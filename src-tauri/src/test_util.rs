//! 테스트 공용: 실제 설정·폴더를 건드리지 않도록 전부 임시 폴더에서 돈다.
//! 설정 위치(SMASH_HOME)와 언어가 전역이라 테스트는 LOCK 으로 하나씩 돈다.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub static LOCK: Mutex<()> = Mutex::new(());

static SANDBOX: OnceLock<PathBuf> = OnceLock::new();

pub fn sandbox() -> PathBuf {
    SANDBOX
        .get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("smash-test-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("SMASH_HOME", dir.join("config"));
            dir
        })
        .clone()
}

pub fn write(p: &Path, content: &str, age_days: u64) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
    if age_days > 0 {
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age_days * 86_400);
        let ft = filetime::FileTime::from_system_time(t);
        filetime::set_file_times(p, ft, ft).unwrap();
    }
}
