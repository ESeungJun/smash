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
use crate::organizer::{apply_plan, scan_target, target_root, Applied};

const EVENT_DEBOUNCE: Duration = Duration::from_millis(1500);
const SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);
const RETRY_WATCH: Duration = Duration::from_secs(30);

pub type OnRun = Box<dyn Fn(&str, &Applied) + Send>;

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
    /// 파일 경로 → 지난번 확인한 크기
    sizes: HashMap<String, u64>,
    sweep_at: Option<Instant>,
    retry_at: Option<Instant>,
    on_run: OnRun,
}

impl Inner {
    fn new(tx: Sender<Msg>, on_run: OnRun) -> Self {
        Inner { tx, watchers: HashMap::new(), timers: HashMap::new(), sizes: HashMap::new(), sweep_at: None, retry_at: None, on_run }
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
        if !wanted.is_empty() && self.sweep_at.is_none() {
            self.sweep_at = Some(Instant::now() + SWEEP_INTERVAL);
        } else if wanted.is_empty() {
            self.sweep_at = None;
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
                // 켜자마자 지금 쌓여 있는 파일도 한 번 본다.
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
    }

    fn schedule(&mut self, id: &str, after: Duration) {
        self.timers.insert(id.to_string(), Instant::now() + after);
    }

    /// 수정 후 settle 시간이 지났고, 지난 확인 때와 크기가 같은 파일만 '다 받아진 파일'로 본다.
    pub fn check(&mut self, id: &str) -> Option<Applied> {
        self.timers.remove(id);
        let root = self.watchers.get(id)?.0.clone();
        let cfg = match load_config() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[auto] {}: {e}", root.display());
                return None;
            }
        };
        if !cfg.targets.iter().any(|t| t.id == id && t.auto) {
            return None;
        }
        let scan = match scan_target(&cfg, Some(id)) {
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
            let Ok(meta) = std::fs::metadata(&item.src) else { continue };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64() * 1000.0)
                .unwrap_or(0.0);
            seen.insert(item.src.clone());
            let prev = self.sizes.insert(item.src.clone(), meta.len());
            if now - mtime >= settle_ms && prev == Some(meta.len()) {
                ready.push(item);
            } else {
                waiting = true;
            }
        }
        // 사라진 파일 기록은 지운다(맵이 계속 커지지 않게).
        let prefix = format!("{}{}", root.display(), std::path::MAIN_SEPARATOR);
        self.sizes.retain(|f, _| !f.starts_with(&prefix) || seen.contains(f));
        if waiting {
            self.schedule(id, Duration::from_secs_f64(settle_ms / 1000.0));
        }
        if ready.is_empty() {
            return None;
        }
        let srcs: Vec<String> = ready.iter().map(|i| i.src.clone()).collect();
        let result = apply_plan(&cfg, ready, "auto");
        for s in srcs {
            self.sizes.remove(&s);
        }
        (self.on_run)(&root.to_string_lossy(), &result);
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
    pub fn start(on_run: OnRun) -> Self {
        let (tx, rx) = mpsc::channel::<Msg>();
        let inner_tx = tx.clone();
        thread::spawn(move || {
            let mut inner = Inner::new(inner_tx, on_run);
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
                            inner.schedule(&id, EVENT_DEBOUNCE);
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
    Inner::new(tx, on_run)
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
        let mut inner = test_inner(Box::new(move |_, _| *r2.lock().unwrap() += 1));
        assert_eq!(inner.sync(None), vec!["inbox"]);
        // 첫 확인: 크기를 기록만 하고 기다린다.
        assert!(inner.check("inbox").is_none());
        assert!(inbox.join("photo.JPG").exists());
        // 기다리는 동안 새로 받는 중인 파일(방금 수정됨)
        write(&inbox.join("downloading.png"), "partial", 0);
        std::thread::sleep(Duration::from_millis(3200));
        inner.check("inbox");
        assert!(*runs.lock().unwrap() >= 1, "정리가 실행됐다");
        assert!(inbox.join("이미지").join("photo.JPG").exists());
        if !cfg!(target_os = "macos") || std::env::var("CI").is_ok() {
            assert!(!inbox.join("old-installer.dmg").exists(), "오래된 설치 파일은 휴지통으로");
        }
        assert!(inbox.join("downloading.png").exists(), "아직 받는 중인 파일은 기다린다");
        assert_eq!(load_history().last().unwrap().source, "auto");
        // 끄면 감시를 멈춘다.
        cfg.targets[0].auto = false;
        let saved = save_config(&cfg).unwrap();
        assert!(inner.sync(Some(saved)).is_empty());
    }
}
