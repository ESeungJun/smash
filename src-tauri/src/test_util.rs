//! 테스트 공용: 실제 설정·폴더를 건드리지 않도록 전부 임시 폴더에서 돈다.
//! 설정 위치(SMASH_HOME)와 언어가 전역이라 테스트는 LOCK 으로 하나씩 돈다.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub static LOCK: Mutex<()> = Mutex::new(());

static SANDBOX: OnceLock<PathBuf> = OnceLock::new();

pub fn sandbox() -> PathBuf {
    SANDBOX
        .get_or_init(|| {
            // 자식 프로세스(도중 종료 흉내)는 부모가 쓰던 임시 폴더를 그대로 쓴다
            if let Some(dir) = std::env::var_os("SMASH_TEST_SANDBOX").map(PathBuf::from) {
                std::env::set_var("SMASH_HOME", dir.join("config"));
                std::env::set_var("SMASH_TRASH_DIR", dir.join("trash"));
                return dir;
            }
            let dir = std::env::temp_dir().join(format!("smash-test-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("SMASH_HOME", dir.join("config"));
            // 실제 휴지통 대신 임시 폴더로 보낸다
            std::env::set_var("SMASH_TRASH_DIR", dir.join("trash"));
            dir
        })
        .clone()
}

/// `age_days` 일 전에 받아서 그 뒤로 그대로인 파일을 만든다(수정 시각·받은 시각 둘 다 그때).
pub fn write(p: &Path, content: &str, age_days: u64) {
    write_arrived(p, content, age_days, Some(age_days));
}

/// 수정 시각과 받은 시각을 따로 정한다. `received_days` 가 None 이면 받은 시각을 모르는 파일.
/// 받은 시각은 OS 가 정하는 값이라 시험에서는 덮어쓴 값을 쓴다(0 일이면 실제 값 그대로).
pub fn write_arrived(p: &Path, content: &str, mtime_days: u64, received_days: Option<u64>) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
    let ago = |days: u64| std::time::SystemTime::now() - std::time::Duration::from_secs(days * 86_400);
    if mtime_days > 0 {
        let ft = filetime::FileTime::from_system_time(ago(mtime_days));
        filetime::set_file_times(p, ft, ft).unwrap();
    }
    let mut map = RECEIVED.lock().unwrap_or_else(|e| e.into_inner());
    match received_days {
        Some(0) => {
            map.remove(p);
        }
        Some(d) => {
            let ms = ago(d).duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64() * 1000.0;
            map.insert(p.to_path_buf(), Some(ms));
        }
        None => {
            map.insert(p.to_path_buf(), None);
        }
    }
}

static RECEIVED: Mutex<std::collections::BTreeMap<PathBuf, Option<f64>>> = Mutex::new(std::collections::BTreeMap::new());

/// 시험에서 정해 둔 받은 시각. 바깥 Some 은 "정해 둠", 안쪽 None 은 "모름".
pub fn received_override(p: &Path) -> Option<Option<f64>> {
    RECEIVED.lock().unwrap_or_else(|e| e.into_inner()).get(p).copied()
}

/// 0 이 아니면 그만큼 항목을 처리한 뒤 프로세스를 그대로 끝낸다(정리 도중 종료 흉내).
pub static ABORT_AFTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static FAULT_SEEN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn fault_point() {
    use std::sync::atomic::Ordering::SeqCst;
    let limit = ABORT_AFTER.load(SeqCst);
    if limit > 0 && FAULT_SEEN.fetch_add(1, SeqCst) >= limit {
        std::process::abort();
    }
}

type MoveHook = Box<dyn Fn(&Path) + Send>;
static BEFORE_MOVE: Mutex<Option<MoveHook>> = Mutex::new(None);

/// 계획과 이동 사이에 끼어드는 다른 프로그램 흉내: 파일을 옮기기 직전에 목적지 경로로 한 번 불린다.
pub fn set_before_move(hook: Option<MoveHook>) {
    *BEFORE_MOVE.lock().unwrap_or_else(|e| e.into_inner()) = hook;
}

pub fn before_move(dest: &Path) {
    if let Some(hook) = BEFORE_MOVE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        hook(dest);
    }
}
