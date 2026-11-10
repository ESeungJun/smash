//! 정리 기록. 실행 하나가 `history/<실행 id>.jsonl` 파일 하나다.
//!
//! - 파일을 옮기기 **전에** 무엇을 옮길지 한 줄 적고 디스크에 내려쓴다(fsync). 도중에 프로세스가 죽거나
//!   전원이 나가도 옮긴 만큼은 기록에 남아 되돌릴 수 있다.
//! - 실행마다 파일이 달라서 앱·자동 정리 스레드·MCP 프로세스가 동시에 정리해도 서로의 기록을 덮지 않는다.
//! - 실행 중에는 `<id>.lock` 을 잠가 둔다. 끝 줄이 없는데 잠금도 없으면 "중단된 정리"다.
//! - 예전 `history.json`(한 파일에 전부)은 읽기만 한다. 그 안의 실행을 되돌릴 때 실행별 파일로 옮겨 적는다.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{config_dir, contract_home, history_path, new_id};
use crate::lang::l;

pub const HISTORY_LIMIT: usize = 50;

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Op {
    #[serde(rename = "type")]
    pub kind: String,
    pub from: String,
    /// 옮긴 자리. 휴지통으로 보낸 것은 휴지통 안 위치(알 수 있을 때만)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub size: u64,
    /// 되돌리기로 원래 자리에 돌아갔는지
    #[serde(skip_serializing_if = "is_false")]
    pub undone: bool,
    /// 기록 파일 안에서의 번호
    #[serde(skip)]
    pub idx: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Failed {
    pub file: String,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Run {
    pub id: String,
    pub at: String,
    pub source: String,
    pub ops: Vec<Op>,
    /// 되돌리기를 했고 더 돌릴 것이 남지 않았다
    pub undone: bool,
    /// 옮기지 못한 파일과 이유
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<Failed>,
    /// "done" · "interrupted"(도중에 끝남) · "running"(지금 실행 중)
    #[serde(skip_serializing_if = "String::is_empty")]
    pub state: String,
    /// 이 실행이 새로 만든 폴더
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dirs: Vec<String>,
    /// 되돌리기를 한 번이라도 시도했는지
    #[serde(skip_serializing_if = "is_false")]
    pub tried: bool,
}

impl Run {
    /// 되돌리기로 다시 시도할 수 있는 것: 옮긴 것과, 휴지통 안 위치를 아는 것
    pub fn retryable(&self) -> usize {
        self.ops.iter().filter(|o| !o.undone && (o.kind == "move" || o.to.is_some())).count()
    }

    fn settle(&mut self) {
        self.undone = self.tried && self.retryable() == 0;
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "t", rename_all = "lowercase")]
enum Line {
    Begin { id: String, at: String, source: String },
    Mkdir { path: String },
    /// 지금부터 할 일. 이 줄을 디스크에 내려쓴 뒤에 파일을 건드린다.
    Op {
        i: usize,
        #[serde(flatten)]
        op: Op,
    },
    /// 휴지통으로 보낸 뒤 알게 된 휴지통 안 위치
    Done { i: usize, to: Option<String> },
    /// 적어 둔 일을 하지 못했다(없던 일)
    Void { i: usize },
    Fail { file: String, reason: String },
    Undo { i: usize },
    Tried { at: String },
    End { at: String },
}

pub fn iso_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn history_dir() -> PathBuf {
    config_dir().join("history")
}

fn run_path(id: &str) -> PathBuf {
    history_dir().join(format!("{id}.jsonl"))
}

fn lock_path(id: &str) -> PathBuf {
    history_dir().join(format!("{id}.lock"))
}

/// 실행 id 는 우리가 만든 것만 받는다(파일 이름으로 쓰므로 경로 문자를 막는다)
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn append(file: &mut File, line: &Line, sync: bool) -> std::io::Result<()> {
    let mut text = serde_json::to_string(line).map_err(std::io::Error::other)?;
    text.push('\n');
    file.write_all(text.as_bytes())?;
    if sync {
        file.sync_data()?;
    }
    Ok(())
}

/// 잠금 파일을 만들고 잠근다. 다른 곳에서 이미 잡고 있으면 None.
fn take_lock(id: &str) -> Option<File> {
    let f = OpenOptions::new().create(true).truncate(false).write(true).open(lock_path(id)).ok()?;
    f.try_lock().ok()?;
    Some(f)
}

fn release_lock(id: &str, lock: Option<File>) {
    if let Some(f) = lock {
        // 쥔 채로 지운 뒤 놓는다(놓고 지우면 그 사이 다른 쪽이 잠근 파일을 지우게 된다)
        let _ = fs::remove_file(lock_path(id));
        drop(f);
    }
}

/// 다른 프로세스·스레드가 이 실행을 쥐고 있는지
fn is_locked(id: &str) -> bool {
    match OpenOptions::new().write(true).open(lock_path(id)) {
        // 읽는 쪽은 공유 잠금으로만 확인한다(읽는 동안 다른 쪽에 "실행 중"으로 보이지 않게)
        Ok(f) => match f.try_lock_shared() {
            Ok(()) => {
                let _ = f.unlock();
                false
            }
            Err(_) => true,
        },
        Err(_) => false,
    }
}

// ── 쓰는 쪽 ───────────────────────────────────────────

/// 정리 한 번의 기록을 쓰면서 남긴다. 아무것도 적지 않으면 파일도 만들지 않는다.
pub struct Recorder {
    id: String,
    at: String,
    source: String,
    file: Option<File>,
    lock: Option<File>,
    ops: Vec<Op>,
    dead: HashSet<usize>,
    failed: Vec<Failed>,
    dirs: Vec<String>,
    /// 기록을 쓰지 못했다. 이 뒤로는 파일을 옮기지 않는다.
    pub error: Option<String>,
    /// (끝낸 수, 전체 수)를 알려 줄 곳
    pub progress: Option<Box<dyn FnMut(usize, usize) + Send>>,
}

impl Recorder {
    pub fn begin(source: &str) -> Self {
        Recorder {
            id: new_id("run"),
            at: iso_now(),
            source: source.into(),
            file: None,
            lock: None,
            ops: Vec::new(),
            dead: HashSet::new(),
            failed: Vec::new(),
            dirs: Vec::new(),
            error: None,
            progress: None,
        }
    }

    fn write(&mut self, line: Line, sync: bool) -> Result<(), String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        let res = (|| -> std::io::Result<()> {
            if self.file.is_none() {
                fs::create_dir_all(history_dir())?;
                self.lock = take_lock(&self.id);
                let mut f = OpenOptions::new().create_new(true).append(true).open(run_path(&self.id))?;
                append(&mut f, &Line::Begin { id: self.id.clone(), at: self.at.clone(), source: self.source.clone() }, false)?;
                self.file = Some(f);
            }
            append(self.file.as_mut().unwrap(), &line, sync)
        })();
        res.map_err(|e| {
            let msg = format!("{}{e}", l("정리 기록을 쓰지 못했습니다: ", "Could not write the history: "));
            self.error = Some(msg.clone());
            msg
        })
    }

    /// 할 일을 적고 디스크에 내려쓴다. 성공해야만 파일을 건드린다.
    pub fn intent(&mut self, mut op: Op) -> Result<usize, String> {
        let i = self.ops.len();
        op.idx = i;
        self.write(Line::Op { i, op: op.clone() }, true)?;
        self.ops.push(op);
        Ok(i)
    }

    /// 휴지통 안 위치를 덧붙인다
    pub fn done(&mut self, i: usize, to: Option<String>) {
        if to.is_some() {
            self.ops[i].to = to.clone();
            let _ = self.write(Line::Done { i, to }, false);
        }
    }

    /// 적어 둔 일을 하지 못했다
    pub fn void(&mut self, i: usize) {
        self.dead.insert(i);
        let _ = self.write(Line::Void { i }, false);
    }

    pub fn fail(&mut self, file: &str, reason: &str) {
        self.failed.push(Failed { file: file.into(), reason: reason.into() });
        let _ = self.write(Line::Fail { file: file.into(), reason: reason.into() }, false);
    }

    pub fn mkdir(&mut self, path: &Path) {
        let path = path.to_string_lossy().into_owned();
        if !self.dirs.contains(&path) {
            self.dirs.push(path.clone());
            let _ = self.write(Line::Mkdir { path }, false);
        }
    }

    /// 끝 줄을 적고 닫는다. 적은 것이 하나도 없으면 None.
    pub fn finish(mut self) -> (Option<Run>, Option<String>) {
        if self.file.is_none() {
            return (None, self.error.take());
        }
        let _ = self.write(Line::End { at: iso_now() }, true);
        self.file = None;
        release_lock(&self.id, self.lock.take());
        let ops: Vec<Op> = self.ops.iter().filter(|o| !self.dead.contains(&o.idx)).cloned().collect();
        let run = Run { id: self.id.clone(), at: self.at.clone(), source: self.source.clone(), ops, failed: self.failed.clone(), state: "done".into(), dirs: self.dirs.clone(), ..Default::default() };
        prune();
        (Some(run), self.error.take())
    }
}

// ── 읽는 쪽 ───────────────────────────────────────────

fn read_run(path: &Path) -> Option<Run> {
    let text = fs::read_to_string(path).ok()?;
    let mut run = Run::default();
    let mut ops: BTreeMap<usize, Op> = BTreeMap::new();
    let mut ended = false;
    for raw in text.lines() {
        // 쓰다가 끊긴 마지막 줄은 버린다
        let Ok(line) = serde_json::from_str::<Line>(raw) else { continue };
        match line {
            Line::Begin { id, at, source } => {
                run.id = id;
                run.at = at;
                run.source = source;
            }
            Line::Mkdir { path } => run.dirs.push(path),
            Line::Op { i, mut op } => {
                op.idx = i;
                ops.insert(i, op);
            }
            Line::Done { i, to } => {
                if let Some(op) = ops.get_mut(&i) {
                    op.to = to;
                }
            }
            Line::Void { i } => {
                ops.remove(&i);
            }
            Line::Fail { file, reason } => run.failed.push(Failed { file, reason }),
            Line::Undo { i } => {
                if let Some(op) = ops.get_mut(&i) {
                    op.undone = true;
                }
            }
            Line::Tried { .. } => run.tried = true,
            Line::End { .. } => ended = true,
        }
    }
    // 파일 이름과 안에 적힌 id 가 같아야 한다(다른 실행인 척하는 파일은 읽지 않는다)
    if !valid_id(&run.id) || path.file_stem().is_none_or(|s| s.to_string_lossy() != run.id) {
        return None;
    }
    run.ops = ops.into_values().collect();
    run.state = if is_locked(&run.id) {
        "running"
    } else if ended {
        "done"
    } else {
        "interrupted"
    }
    .into();
    run.settle();
    Some(run)
}

/// 예전 `history.json`. 읽기만 하고 고쳐 쓰지 않는다. 깨져 있으면 빈 목록(파일은 그대로 둔다).
fn load_legacy() -> Vec<Run> {
    let Ok(text) = fs::read_to_string(history_path()) else { return vec![] };
    let Ok(list) = serde_json::from_str::<Vec<Value>>(&text) else { return vec![] };
    list.into_iter()
        .filter_map(|v| serde_json::from_value::<Run>(v).ok())
        .map(|mut r| {
            let undone = r.undone;
            for (i, op) in r.ops.iter_mut().enumerate() {
                op.idx = i;
                op.undone = undone && op.kind == "move";
            }
            r.tried = undone;
            r.state = "done".into();
            r
        })
        .collect()
}

fn run_files() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(history_dir()) else { return vec![] };
    entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "jsonl")).collect()
}

/// 오래된 것부터. 최근 50건.
pub fn load_history() -> Vec<Run> {
    let mut runs: Vec<Run> = run_files().iter().filter_map(|p| read_run(p)).collect();
    let ids: HashSet<String> = runs.iter().map(|r| r.id.clone()).collect();
    runs.extend(load_legacy().into_iter().filter(|r| !ids.contains(&r.id)));
    runs.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
    let start = runs.len().saturating_sub(HISTORY_LIMIT);
    runs.split_off(start)
}

/// 50건을 넘는 오래된 실행 파일을 지운다. 지금 실행 중인 것은 건드리지 않는다.
fn prune() {
    let mut runs: Vec<Run> = run_files().iter().filter_map(|p| read_run(p)).collect();
    if runs.len() <= HISTORY_LIMIT {
        return;
    }
    runs.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
    let extra = runs.len() - HISTORY_LIMIT;
    for r in runs.iter().take(extra).filter(|r| r.state != "running") {
        let _ = fs::remove_file(run_path(&r.id));
        let _ = fs::remove_file(lock_path(&r.id));
    }
}

// ── 되돌리기 ──────────────────────────────────────────

fn nothing_to_undo() -> Value {
    json!({ "ok": false, "message": l("되돌릴 작업이 없습니다", "Nothing to undo"), "restored": 0, "failed": [], "trashed": [] })
}

/// 예전 기록의 실행을 실행별 파일로 옮겨 적는다(되돌린 결과를 덧붙일 수 있게).
fn adopt_legacy(run: &Run) -> std::io::Result<()> {
    fs::create_dir_all(history_dir())?;
    let mut f = OpenOptions::new().create_new(true).append(true).open(run_path(&run.id))?;
    append(&mut f, &Line::Begin { id: run.id.clone(), at: run.at.clone(), source: run.source.clone() }, false)?;
    for op in &run.ops {
        append(&mut f, &Line::Op { i: op.idx, op: Op { undone: false, ..op.clone() } }, false)?;
    }
    append(&mut f, &Line::End { at: run.at.clone() }, true)
}

/// 실행 하나를 되돌린다. `id` 가 없으면 아직 돌릴 것이 남은 가장 최근 실행.
/// 옮긴 파일은 원래 자리로, 휴지통으로 보낸 파일은 휴지통 안 위치를 알면 꺼내 온다.
/// 못 돌린 것은 기록에 그대로 남아 다시 시도할 수 있다.
pub fn undo_run(id: Option<&str>) -> Value {
    let runs = load_history();
    let pick = |r: &&Run| r.state != "running" && !r.undone && !r.ops.is_empty();
    let found = match id.filter(|s| !s.is_empty()) {
        Some(id) => runs.iter().find(|r| r.id == id),
        None => runs.iter().rev().find(pick),
    };
    let Some(run) = found else { return nothing_to_undo() };
    if run.state == "running" {
        return json!({ "ok": false, "message": l("아직 정리하는 중입니다", "That run is still in progress"), "restored": 0, "failed": [], "trashed": [] });
    }
    if run.undone || run.ops.is_empty() || !valid_id(&run.id) {
        return nothing_to_undo();
    }
    let fail = |e: String| json!({ "ok": false, "message": format!("{}{e}", l("기록을 쓰지 못해 되돌리지 않았습니다: ", "Could not write the history, nothing was undone: ")), "restored": 0, "failed": [], "trashed": [] });
    if !run_path(&run.id).exists() {
        if let Err(e) = adopt_legacy(run) {
            return fail(e.to_string());
        }
    }
    let Some(lock) = take_lock(&run.id) else {
        return json!({ "ok": false, "message": l("다른 곳에서 이 정리를 처리하는 중입니다", "Another process is working on this run"), "restored": 0, "failed": [], "trashed": [] });
    };
    let mut file = match OpenOptions::new().append(true).open(run_path(&run.id)) {
        Ok(f) => f,
        Err(e) => {
            release_lock(&run.id, Some(lock));
            return fail(e.to_string());
        }
    };

    let exists = |p: &str| fs::symlink_metadata(p).is_ok();
    // 기록 파일은 다른 프로그램이 고칠 수 있다. 되돌릴 때도 지금 설정의 정리 대상 안에서만, 일반 파일만 옮긴다.
    let cfg = crate::config::load_config().ok();
    let inside = |p: &str| {
        let p = Path::new(p);
        cfg.as_ref().is_some_and(|c| crate::organizer::inside_any_root(c, p) && p.parent().is_some_and(|d| crate::organizer::really_inside_any_root(c, d)))
    };
    let is_file = |p: &str| fs::symlink_metadata(p).is_ok_and(|m| m.is_file());
    let outside = || l("정리 대상 폴더 밖이라 되돌리지 않았습니다", "Outside the folders you set up, so it was not undone");
    let not_file = || l("파일이 아닙니다", "Not a file");
    let (mut restored, mut from_trash, mut left) = (0, 0, 0);
    let mut failed = Vec::new();
    let mut trashed = Vec::new();
    for op in run.ops.iter().rev().filter(|o| !o.undone) {
        let to = op.to.clone().unwrap_or_default();
        // Ok(true) 돌려놨다 · Ok(false) 이미 원래 자리에 있다 · Err 못 돌렸다
        let res: Result<bool, String> = if op.kind == "move" {
            if exists(&to) && !(inside(&to) && inside(&op.from)) {
                Err(outside())
            } else if exists(&to) && !is_file(&to) {
                Err(not_file())
            } else if exists(&to) {
                crate::fsops::move_file(Path::new(&to), Path::new(&op.from)).map(|_| true).map_err(|e| match e.kind() {
                    std::io::ErrorKind::AlreadyExists => l("원래 자리에 같은 이름 파일이 있습니다", "A file with the same name is in the original place"),
                    _ => e.to_string(),
                })
            } else if exists(&op.from) {
                Ok(false)
            } else {
                Err(l("옮긴 파일이 없습니다", "The moved file is gone"))
            }
        } else if exists(&op.from) && !to.is_empty() && exists(&to) {
            // 휴지통에 그대로 있는데 원래 자리에 같은 이름의 다른 파일이 생겼다
            Err(l("원래 자리에 같은 이름 파일이 있습니다", "A file with the same name is in the original place"))
        } else if exists(&op.from) {
            // 휴지통으로 가지 않았거나 사용자가 이미 꺼냈다
            Ok(false)
        } else if to.is_empty() {
            trashed.push(contract_home(&op.from));
            continue;
        } else if !inside(&op.from) {
            Err(outside())
        } else if exists(&to) && !is_file(&to) {
            Err(not_file())
        } else {
            crate::trash::restore_from_trash(Path::new(&to), Path::new(&op.from)).map(|_| {
                from_trash += 1;
                true
            })
        };
        match res {
            Ok(moved) => {
                // 돌려놓은 것은 바로 적는다. 못 적으면 여기서 멈춘다(같은 파일을 두 번 옮기지 않게).
                if let Err(e) = append(&mut file, &Line::Undo { i: op.idx }, true) {
                    failed.push(json!({ "file": contract_home(&op.from), "reason": e.to_string() }));
                    break;
                }
                restored += moved as usize;
            }
            Err(reason) => {
                left += 1;
                let shown = if op.kind == "move" { &to } else { &op.from };
                failed.push(json!({ "file": contract_home(shown), "reason": reason }));
            }
        }
    }
    let _ = append(&mut file, &Line::Tried { at: iso_now() }, true);
    drop(file);
    // 이 실행이 만든 폴더 중 비어 있는 것은 지운다(깊은 쪽부터). 비어 있지 않으면 그대로 남는다.
    let mut dirs = run.dirs.clone();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.len()));
    let removed_dirs = dirs.iter().filter(|d| cfg.as_ref().is_some_and(|c| crate::organizer::within_roots(c, Path::new(d))) && fs::remove_dir(d).is_ok()).count();
    release_lock(&run.id, Some(lock));
    json!({
        "ok": true,
        "runId": run.id,
        "restored": restored,
        "restoredFromTrash": from_trash,
        "failed": failed,
        "left": left,
        "trashed": trashed,
        "removedDirs": removed_dirs,
    })
}

pub fn undo_last() -> Value {
    undo_run(None)
}

/// 최근 7일 동안 치운 양 (기록 탭 맨 위 한 줄, 주간 알림)
pub fn week_stats(now_ms: f64) -> Value {
    let since = chrono::DateTime::from_timestamp_millis((now_ms - 7.0 * 86_400_000.0) as i64).map(|d| d.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()).unwrap_or_default();
    let (mut moved, mut trashed, mut trash_bytes, mut runs) = (0u64, 0u64, 0u64, 0u64);
    for r in load_history().iter().filter(|r| r.at >= since) {
        let live: Vec<&Op> = r.ops.iter().filter(|o| !o.undone).collect();
        if live.is_empty() {
            continue;
        }
        runs += 1;
        for o in live {
            if o.kind == "move" {
                moved += 1;
            } else {
                trashed += 1;
                trash_bytes += o.size;
            }
        }
    }
    json!({ "runs": runs, "moved": moved, "trashed": trashed, "trashBytes": trash_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{sandbox, write, LOCK};

    fn clean() -> PathBuf {
        let sb = sandbox();
        let _ = fs::remove_dir_all(sb.join("config"));
        sb
    }

    fn mv(from: &Path, to: &Path) -> Op {
        Op { kind: "move".into(), from: from.to_string_lossy().into_owned(), to: Some(to.to_string_lossy().into_owned()), ..Default::default() }
    }

    #[test]
    fn journal_is_written_before_the_move() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sb = clean();
        let (a, b) = (sb.join("j").join("a.txt"), sb.join("j").join("out").join("a.txt"));
        let mut rec = Recorder::begin("test");
        let i = rec.intent(mv(&a, &b)).unwrap();
        // 끝내기 전(=실행 중)에도 다른 쪽에서 읽을 수 있고, 실행 중으로 보인다
        let seen = load_history();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].state, "running");
        assert_eq!(seen[0].ops[0].idx, i);
        assert_eq!(undo_run(Some(&seen[0].id))["ok"], json!(false), "실행 중인 것은 되돌리지 않는다");
        // 끝 줄 없이 손을 놓으면(프로세스가 죽은 것과 같다) 중단된 정리로 남는다
        let id = seen[0].id.clone();
        drop(rec);
        let seen = load_history();
        assert_eq!((seen[0].id.as_str(), seen[0].state.as_str()), (id.as_str(), "interrupted"));
    }

    #[test]
    fn void_fail_and_empty_runs() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sb = clean();
        assert!(Recorder::begin("test").finish().0.is_none(), "아무 일도 없으면 기록도 없다");
        assert!(load_history().is_empty());
        let mut rec = Recorder::begin("test");
        let a = rec.intent(mv(&sb.join("x"), &sb.join("y"))).unwrap();
        let b = rec.intent(Op { kind: "trash".into(), from: "/t/z".into(), ..Default::default() }).unwrap();
        rec.void(a);
        rec.done(b, Some("/trash/z".into()));
        rec.fail("x", "안 됨");
        let run = rec.finish().0.unwrap();
        assert_eq!(run.ops.len(), 1);
        let back = load_history().pop().unwrap();
        assert_eq!(back.state, "done");
        assert_eq!(back.ops, run.ops);
        assert_eq!(back.ops[0].to.as_deref(), Some("/trash/z"));
        assert_eq!(back.failed, vec![Failed { file: "x".into(), reason: "안 됨".into() }]);
    }

    #[test]
    fn torn_last_line_is_ignored() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sb = clean();
        let mut rec = Recorder::begin("test");
        rec.intent(mv(&sb.join("a"), &sb.join("b"))).unwrap();
        let id = rec.id.clone();
        drop(rec);
        let mut f = OpenOptions::new().append(true).open(run_path(&id)).unwrap();
        f.write_all(b"{\"t\":\"op\",\"i\":1,\"type\":\"mo").unwrap();
        let runs = load_history();
        assert_eq!(runs[0].ops.len(), 1, "쓰다 만 줄은 버리고 앞의 것은 살린다");
    }

    #[test]
    fn legacy_history_is_read_and_undone_without_rewriting() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sb = clean();
        let dir = sb.join("legacy");
        let _ = fs::remove_dir_all(&dir);
        write(&dir.join("문서").join("a.pdf"), "a", 0);
        crate::config::save_config_value(&json!({ "targets": [{ "id": "legacy", "path": dir, "rules": [] }] })).unwrap();
        let from = dir.join("a.pdf").to_string_lossy().into_owned();
        let to = dir.join("문서").join("a.pdf").to_string_lossy().into_owned();
        let legacy = json!([
            { "id": "run_old1", "at": "2026-11-01T00:00:00.000Z", "source": "app", "undone": true, "ops": [{ "type": "move", "from": "/x", "to": "/y" }] },
            { "id": "run_old2", "at": "2026-11-02T00:00:00.000Z", "source": "auto", "undone": false, "ops": [{ "type": "move", "from": from, "to": to }, { "type": "trash", "from": "/gone/b.dmg" }] }
        ]);
        fs::create_dir_all(config_dir()).unwrap();
        fs::write(history_path(), serde_json::to_string(&legacy).unwrap()).unwrap();
        let before = fs::read(history_path()).unwrap();
        let runs = load_history();
        assert_eq!(runs.iter().map(|r| (r.id.as_str(), r.undone)).collect::<Vec<_>>(), vec![("run_old1", true), ("run_old2", false)]);
        let res = undo_last();
        assert_eq!(res["runId"], json!("run_old2"));
        assert_eq!(res["restored"], json!(1));
        assert_eq!(res["trashed"].as_array().unwrap().len(), 1, "위치를 모르는 휴지통 파일은 알려 주기만 한다");
        assert!(dir.join("a.pdf").exists());
        assert_eq!(fs::read(history_path()).unwrap(), before, "예전 기록 파일은 고쳐 쓰지 않는다");
        let runs = load_history();
        assert_eq!(runs.len(), 2, "옮겨 적은 실행이 두 번 나오지 않는다");
        assert!(runs[1].undone);
        assert_eq!(undo_last()["ok"], json!(false));
        // 깨진 예전 기록은 무시하고(그대로 두고) 새 기록은 계속 남는다
        fs::write(history_path(), "{ 깨짐").unwrap();
        assert_eq!(load_history().len(), 1);
        assert_eq!(fs::read_to_string(history_path()).unwrap(), "{ 깨짐");
    }

    #[test]
    fn keeps_only_recent_runs_and_rejects_odd_ids() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sb = clean();
        for _ in 0..HISTORY_LIMIT + 3 {
            let mut rec = Recorder::begin("test");
            rec.intent(mv(&sb.join("a"), &sb.join("b"))).unwrap();
            rec.finish();
        }
        assert_eq!(run_files().len(), HISTORY_LIMIT);
        assert_eq!(load_history().len(), HISTORY_LIMIT);
        assert_eq!(undo_run(Some("../../etc/passwd"))["ok"], json!(false));
        assert!(!valid_id("a/b") && !valid_id("") && valid_id("run_mwaep0o52t"));
    }
}
