//! 자동 정리: 켜 둔 폴더에 새 파일이 들어오면, 다 받아질 때까지 기다렸다가 규칙대로 정리한다.
//! AI 제안은 쓰지 않고 사용자가 만든 규칙만 적용한다.
//! 감시·타이머는 작업 스레드 하나가 맡는다 (Electron 버전의 setTimeout 묶음과 같은 동작).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::config::{load_config, now_ms, Config};
use crate::organizer::{apply_plan, scan_target_with, target_root, Applied};

const EVENT_DEBOUNCE: Duration = Duration::from_millis(1500);
/// 이벤트가 계속 와도(큰 파일을 받는 중) 첫 이벤트부터 이만큼 지나면 한 번은 확인한다
const EVENT_MAX_WAIT: Duration = Duration::from_secs(15);
const SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);
const FIRST_SWEEP: Duration = Duration::from_secs(2 * 60);
const RETRY_WATCH: Duration = Duration::from_secs(30);
/// 크기가 그대로인 것을 이만큼 연속으로 확인해야 다 받아진 파일로 본다
const STABLE_CHECKS: u8 = 2;
/// 이보다 큰 파일은 더 오래 기다린다(크기를 미리 잡아 두고 천천히 채우는 프로그램이 있다)
const BIG_FILE: u64 = 1 << 30;
const BIG_FILE_SETTLE_MS: f64 = 60_000.0;
/// 받는 중인 파일 옆에 생기는 임시 파일의 확장자
const PARTIAL_EXTS: [&str; 3] = ["part", "crdownload", "download"];

/// 자동 정리를 한 폴더의 id 와 결과
pub type OnRun = Box<dyn Fn(&str, &Applied) + Send>;
/// 10분마다 한 번 (잔소리·주간 알림을 여기서 판단한다)
pub type OnSweep = Box<dyn Fn() + Send>;

/// 받는 중인 파일인지: 같은 폴더에 `<이름>.part` · `.crdownload` · `.download` 가 있으면 아직 받는 중이다.
fn has_partial_sibling(file: &str) -> bool {
    PARTIAL_EXTS.iter().any(|ext| std::fs::symlink_metadata(format!("{file}.{ext}")).is_ok())
}

enum Msg {
    Sync(Option<Config>),
    Event(String),
    WatchError(String, String),
    Stop,
}

/// 감시·타이머 상태. 작업 스레드 안에서만 쓴다 (테스트는 직접 부른다)
pub struct Inner {
    tx: Sender<Msg>,
    watchers: HashMap<String, (PathBuf, RecommendedWatcher)>,
    timers: HashMap<String, Instant>,
    /// 폴더 id → 아직 확인하지 않은 첫 이벤트 시각
    first_event: HashMap<String, Instant>,
    /// 파일 경로 → (지난번 확인한 크기, 그 크기가 연속으로 같았던 횟수)
    sizes: HashMap<String, (u64, u8)>,
    sweep_at: Option<Instant>,
    retry_at: Option<Instant>,
    on_run: OnRun,
    on_sweep: OnSweep,
}

impl Inner {
    fn new(tx: Sender<Msg>, on_run: OnRun, on_sweep: OnSweep) -> Self {
        Inner {
            tx,
            watchers: HashMap::new(),
            timers: HashMap::new(),
            first_event: HashMap::new(),
            sizes: HashMap::new(),
            // 자동 정리를 켠 폴더가 없어도 주기 확인은 돈다(잔소리·주간 알림)
            sweep_at: Some(Instant::now() + FIRST_SWEEP),
            retry_at: None,
            on_run,
            on_sweep,
        }
    }

    /// 설정이 바뀔 때마다 부른다. 자동 정리가 켜진 폴더만 감시한다.
    pub fn sync(&mut self, cfg: Option<Config>) -> Vec<String> {
        let cfg = match cfg.map(Ok).unwrap_or_else(load_config) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[auto] 설정을 읽지 못했습니다: {e}");
                return self.watchers.keys().cloned().collect();
            }
        };
        let wanted: HashMap<String, PathBuf> = cfg.targets.iter().filter(|t| t.auto).map(|t| (t.id.clone(), target_root(t))).collect();
        let stale: Vec<String> = self.watchers.iter().filter(|(id, (root, _))| wanted.get(*id) != Some(root)).map(|(id, _)| id.clone()).collect();
        for id in stale {
            self.unwatch(&id);
        }
        for (id, root) in &wanted {
            if !self.watchers.contains_key(id) {
                self.watch(id, root);
            }
        }
        if self.sweep_at.is_none() {
            self.sweep_at = Some(Instant::now() + SWEEP_INTERVAL);
        }
        let mut ids: Vec<String> = self.watchers.keys().cloned().collect();
        ids.sort();
        ids
    }

    fn watch(&mut self, id: &str, root: &Path) {
        let tx = self.tx.clone();
        let wid = id.to_string();
        let made = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let _ = match res {
                Ok(_) => tx.send(Msg::Event(wid.clone())),
                Err(e) => tx.send(Msg::WatchError(wid.clone(), e.to_string())),
            };
        });
        match made.and_then(|mut w| w.watch(root, RecursiveMode::NonRecursive).map(|_| w)) {
            Ok(w) => {
                self.watchers.insert(id.to_string(), (root.to_path_buf(), w));
                // 켜자마자 지금 쌓여 있는 파일도 한 번 본다. 화면에서 켤 때는 무엇을 치울지 먼저 보여 준 뒤다.
                self.schedule(id, EVENT_DEBOUNCE);
            }
            Err(e) => eprintln!("[auto] {}: {e}", root.display()),
        }
    }

    fn unwatch(&mut self, id: &str) {
        if let Some((root, _w)) = self.watchers.remove(id) {
            let prefix = format!("{}{}", root.display(), std::path::MAIN_SEPARATOR);
            self.sizes.retain(|f, _| !f.starts_with(&prefix));
        }
        self.timers.remove(id);
        self.first_event.remove(id);
    }

    fn schedule(&mut self, id: &str, after: Duration) {
        self.timers.insert(id.to_string(), Instant::now() + after);
    }

    /// 파일 이벤트: 마지막 이벤트 1.5초 뒤에 확인하되, 이벤트가 끊이지 않아도 첫 이벤트 15초 뒤에는 확인한다.
    fn on_event(&mut self, id: &str) -> Instant {
        let now = Instant::now();
        let first = *self.first_event.entry(id.to_string()).or_insert(now);
        let at = (now + EVENT_DEBOUNCE).min(first + EVENT_MAX_WAIT);
        self.timers.insert(id.to_string(), at);
        at
    }

    /// 수정 후 settle 시간이 지났고, 크기가 연속 두 번 그대로인 파일만 '다 받아진 파일'로 본다.
    /// 크기 0 인 파일과, 옆에 받는 중 임시 파일이 있는 파일은 건드리지 않는다.
    pub fn check(&mut self, id: &str) -> Option<Applied> {
        self.timers.remove(id);
        self.first_event.remove(id);
        let root = self.watchers.get(id)?.0.clone();
        let cfg = match load_config() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[auto] {}: {e}", root.display());
                return None;
            }
        };
        let target = cfg.targets.iter().find(|t| t.id == id && t.auto)?;
        let auto_trash = target.auto_trash;
        let scan = match scan_target_with(&cfg, Some(id), false) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[auto] {}: {e}", root.display());
                return None;
            }
        };
        let settle_ms = cfg.settings.auto_delay_sec * 1000.0;
        let now = now_ms();
        let mut ready = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut waiting = false;
        for item in scan.items {
            // 휴지통 규칙을 자동 정리에서 뺀 폴더: 규칙으로 버리는 것만 건너뛴다(같은 파일이 이미 있어 버리는 중복은 그대로)
            if !auto_trash && item.action == "trash" && item.note.is_none() {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&item.src) else { continue };
            // 받는 프로그램이 먼저 만들어 둔 빈 자리표시 파일이거나, 옆에 받는 중 파일이 있다
            if meta.len() == 0 || has_partial_sibling(&item.src) {
                self.sizes.remove(&item.src);
                continue;
            }
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64() * 1000.0)
                .unwrap_or(0.0);
            seen.insert(item.src.clone());
            let stable = match self.sizes.get(&item.src) {
                Some((size, n)) if *size == meta.len() => n.saturating_add(1),
                _ => 0,
            };
            self.sizes.insert(item.src.clone(), (meta.len(), stable));
            let settle = if meta.len() >= BIG_FILE { settle_ms.max(BIG_FILE_SETTLE_MS) } else { settle_ms };
            if now - mtime >= settle && stable >= STABLE_CHECKS {
                ready.push(item);
            } else {
                waiting = true;
            }
        }
        // 사라진 파일 기록은 지운다(맵이 계속 커지지 않게).
        let prefix = format!("{}{}", root.display(), std::path::MAIN_SEPARATOR);
        self.sizes.retain(|f, _| !f.starts_with(&prefix) || seen.contains(f));
        if waiting {
            // 확인을 두 번 더 해야 하므로 기다리는 시간의 절반마다 본다
            self.schedule(id, Duration::from_secs_f64((settle_ms / 2000.0).max(EVENT_DEBOUNCE.as_secs_f64())));
        }
        if ready.is_empty() {
            return None;
        }
        let srcs: Vec<String> = ready.iter().map(|i| i.src.clone()).collect();
        let result = apply_plan(&cfg, ready, "auto");
        for s in srcs {
            self.sizes.remove(&s);
        }
        (self.on_run)(id, &result);
        Some(result)
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.timers.values().copied().chain(self.sweep_at).chain(self.retry_at).min()
    }

    fn run_due(&mut self) {
        let now = Instant::now();
        if self.retry_at.is_some_and(|t| t <= now) {
            self.retry_at = None;
            self.sync(None);
        }
        if self.sweep_at.is_some_and(|t| t <= now) {
            self.sweep_at = Some(now + SWEEP_INTERVAL);
            let ids: Vec<String> = self.watchers.keys().cloned().collect();
            for id in ids {
                self.schedule(&id, Duration::ZERO);
            }
            (self.on_sweep)();
        }
        let due: Vec<String> = self.timers.iter().filter(|(_, t)| **t <= now).map(|(id, _)| id.clone()).collect();
        for id in due {
            self.check(&id);
        }
    }
}

/// 앱이 쥐고 있는 손잡이. 설정이 바뀌면 sync, 끝낼 때 stop
pub struct AutoTidy {
    tx: Sender<Msg>,
}

impl AutoTidy {
    pub fn start(on_run: OnRun, on_sweep: OnSweep) -> Self {
        let (tx, rx) = mpsc::channel::<Msg>();
        let inner_tx = tx.clone();
        thread::spawn(move || {
            let mut inner = Inner::new(inner_tx, on_run, on_sweep);
            loop {
                let msg = match inner.next_deadline() {
                    Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                        Ok(m) => Some(m),
                        Err(RecvTimeoutError::Timeout) => None,
                        Err(RecvTimeoutError::Disconnected) => break,
                    },
                    None => match rx.recv() {
                        Ok(m) => Some(m),
                        Err(_) => break,
                    },
                };
                match msg {
                    Some(Msg::Sync(cfg)) => {
                        inner.sync(cfg);
                    }
                    Some(Msg::Event(id)) => {
                        if inner.watchers.contains_key(&id) {
                            inner.on_event(&id);
                        }
                    }
                    Some(Msg::WatchError(id, e)) => {
                        eprintln!("[auto] {id}: {e}");
                        inner.unwatch(&id);
                        inner.retry_at = Some(Instant::now() + RETRY_WATCH);
                    }
                    Some(Msg::Stop) => {
                        let ids: Vec<String> = inner.watchers.keys().cloned().collect();
                        for id in ids {
                            inner.unwatch(&id);
                        }
                        inner.sweep_at = None;
                        inner.retry_at = None;
                        inner.timers.clear();
                    }
                    None => {}
                }
                inner.run_due();
            }
        });
        AutoTidy { tx }
    }

    pub fn sync(&self, cfg: Option<Config>) {
        let _ = self.tx.send(Msg::Sync(cfg));
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Msg::Stop);
    }
}

#[cfg(test)]
pub fn test_inner(on_run: OnRun) -> Inner {
    let (tx, _rx) = mpsc::channel();
    // 이벤트는 테스트에서 쓰지 않으니 받는 쪽은 버린다
    Inner::new(tx, on_run, Box::new(|| {}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::save_config;
    use crate::lang::set_lang;
    use crate::organizer::load_history;
    use crate::test_util::{sandbox, write, LOCK};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[test]
    fn only_finished_files_are_tidied() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_lang("ko");
        let sb = sandbox();
        let inbox = sb.join("inbox");
        let _ = std::fs::remove_dir_all(&inbox);
        let _ = std::fs::remove_dir_all(sb.join("config"));
        write(&inbox.join("photo.JPG"), "img", 0);
        write(&inbox.join("old-installer.dmg"), "dmg", 30);
        let mut cfg = crate::config::save_config_value(&json!({
            "targets": [{ "id": "inbox", "path": inbox, "auto": true, "rules": [
                { "name": "오래된 설치 파일", "match": { "extensions": ["dmg"], "olderThanDays": 7 }, "action": "trash" },
                { "name": "이미지", "match": { "extensions": ["jpg", "png"] }, "action": "move", "dest": "이미지" }
            ]}],
            "settings": { "autoDelaySec": 3 }
        }))
        .unwrap();
        // 사진은 오래된 파일로 만든다(받은 지 오래됨)
        write(&inbox.join("photo.JPG"), "img", 1);

        let runs = Arc::new(Mutex::new(0));
        let r2 = runs.clone();
        let mut inner = test_inner(Box::new(move |id, _| {
            assert_eq!(id, "inbox");
            *r2.lock().unwrap() += 1;
        }));
        assert_eq!(inner.sync(None), vec!["inbox"]);
        // 첫 확인: 크기를 기록만 하고 기다린다.
        assert!(inner.check("inbox").is_none());
        assert!(inbox.join("photo.JPG").exists());
        // 기다리는 동안 새로 받는 중인 파일(방금 수정됨)
        write(&inbox.join("downloading.png"), "partial", 0);
        // 받는 프로그램이 먼저 만들어 둔 빈 파일, 옆에 .part 가 있는 파일
        write(&inbox.join("placeholder.png"), "", 1);
        write(&inbox.join("movie.png"), "half", 1);
        write(&inbox.join("movie.png.part"), "rest", 0);
        // 두 번째 확인: 크기가 한 번 같았을 뿐이라 아직 기다린다
        assert!(inner.check("inbox").is_none(), "한 번 같은 것으로는 옮기지 않는다");
        assert!(inbox.join("photo.JPG").exists());
        std::thread::sleep(Duration::from_millis(3200));
        inner.check("inbox");
        assert_eq!(*runs.lock().unwrap(), 1, "정리가 실행됐다");
        assert!(inbox.join("이미지").join("photo.JPG").exists());
        assert!(!inbox.join("old-installer.dmg").exists(), "받은 지 오래된 설치 파일은 휴지통으로");
        assert!(sandbox().join("trash").join("old-installer.dmg").exists(), "시험에서는 임시 휴지통으로 간다");
        assert!(inbox.join("downloading.png").exists(), "아직 받는 중인 파일은 기다린다");
        assert!(inbox.join("placeholder.png").exists(), "크기 0 인 자리표시 파일은 건드리지 않는다");
        assert!(inbox.join("movie.png").exists(), "옆에 .part 가 있으면 받는 중으로 본다");
        assert_eq!(load_history().last().unwrap().source, "auto");
        // 다 받아져 .part 가 없어지면 그때 정리된다
        std::fs::remove_file(inbox.join("movie.png.part")).unwrap();
        for _ in 0..3 {
            inner.check("inbox");
        }
        assert!(inbox.join("이미지").join("movie.png").exists());
        // 끄면 감시를 멈춘다.
        cfg.targets[0].auto = false;
        let saved = save_config(&cfg).unwrap();
        assert!(inner.sync(Some(saved)).is_empty());
    }

    #[test]
    fn trash_rules_can_be_left_out_of_auto() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_lang("ko");
        let sb = sandbox();
        let inbox = sb.join("auto-notrash");
        let _ = std::fs::remove_dir_all(&inbox);
        let _ = std::fs::remove_dir_all(sb.join("config"));
        write(&inbox.join("old.dmg"), "dmg", 30);
        write(&inbox.join("photo.jpg"), "img", 1);
        crate::config::save_config_value(&json!({
            "targets": [{ "id": "inbox", "path": inbox, "auto": true, "autoTrash": false, "rules": [
                { "name": "오래된 설치 파일", "match": { "extensions": ["dmg"], "olderThanDays": 7 }, "action": "trash" },
                { "name": "이미지", "match": { "extensions": ["jpg"] }, "action": "move", "dest": "이미지" }
            ]}],
            "settings": { "autoDelaySec": 3 }
        }))
        .unwrap();
        let mut inner = test_inner(Box::new(|_, _| {}));
        inner.sync(None);
        for _ in 0..3 {
            inner.check("inbox");
        }
        assert!(inbox.join("이미지").join("photo.jpg").exists());
        assert!(inbox.join("old.dmg").exists(), "휴지통 규칙을 뺐으면 자동 정리는 버리지 않는다");
    }

    #[test]
    fn busy_folder_is_still_checked() {
        let mut inner = test_inner(Box::new(|_, _| {}));
        let start = Instant::now();
        let first = inner.on_event("x");
        assert!(first <= start + EVENT_DEBOUNCE + Duration::from_millis(50));
        // 이벤트가 끊이지 않아도 첫 이벤트 15초 뒤를 넘겨 미루지 않는다
        inner.first_event.insert("x".into(), start - Duration::from_secs(14));
        let capped = inner.on_event("x");
        assert!(capped <= start + Duration::from_secs(1) + Duration::from_millis(50), "{:?}", capped - start);
        inner.first_event.insert("x".into(), start - Duration::from_secs(60));
        assert!(inner.on_event("x") <= Instant::now(), "이미 넘겼으면 바로 확인한다");
    }
}
