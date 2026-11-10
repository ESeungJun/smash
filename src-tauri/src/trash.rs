//! OS 휴지통으로 보내기와 되돌리기. 영구 삭제는 하지 않는다.
//!
//! 되돌리기 범위:
//! - macOS: 휴지통 안 위치를 받아 두므로 그 자리에서 원래 자리로 옮긴다. 사용자가 휴지통을 비웠거나
//!   휴지통 안에서 옮겼으면 못 돌린다. 휴지통 폴더 접근이 막힌 환경에서도 실패하며, 그때는 그대로 둔다.
//! - Linux(freedesktop): 같은 방식.
//! - Windows: 휴지통 안 위치를 알 수 없어 되돌리지 못한다. 사용자가 휴지통에서 직접 꺼내야 한다.

use std::path::{Path, PathBuf};

use crate::lang::l;

fn failed() -> String {
    l("휴지통으로 옮기지 못했습니다", "Could not move to Trash")
}

/// 시험·개발 실행에서 실제 휴지통 대신 쓸 폴더 (`SMASH_TRASH_DIR`)
fn override_dir() -> Option<PathBuf> {
    std::env::var_os("SMASH_TRASH_DIR").filter(|d| !d.is_empty()).map(PathBuf::from)
}

fn trash_into(dir: &Path, file: &Path) -> Result<Option<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut dest = dir.join(&name);
    let mut i = 2;
    loop {
        match crate::fsops::move_file(file, &dest) {
            Ok(()) => return Ok(Some(dest)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                dest = dir.join(format!("{name} {i}"));
                i += 1;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// 한 번에 넘기는 파일 수. 명령줄 길이 제한 안쪽으로 나눈다.
#[cfg(any(target_os = "macos", test))]
const BATCH: usize = 200;

/// NSFileManager 를 직접 불러서 Finder 자동화 권한 없이 휴지통으로 보낸다.
/// 파일마다 프로세스를 띄우지 않고 묶어서 넘기고, 휴지통 안에서의 경로를 받아 온다.
#[cfg(target_os = "macos")]
fn os_trash(files: &[PathBuf]) -> Vec<Result<Option<PathBuf>, String>> {
    const SCRIPT: &str = "function run(argv){ObjC.import(\"Foundation\");var fm=$.NSFileManager.defaultManager;var out=[];\
for(var i=0;i<argv.length;i++){var res=$();var ok=fm.trashItemAtURLResultingItemURLError($.NSURL.fileURLWithPath(argv[i]),res,null);\
var p=null;if(ok){try{p=ObjC.unwrap(res.path)}catch(e){p=\"\"}}out.push(p);}return JSON.stringify(out)}";
    let mut out = Vec::with_capacity(files.len());
    for chunk in files.chunks(BATCH) {
        let ran = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", SCRIPT]).args(chunk).output();
        let parsed: Vec<Option<String>> = ran
            .ok()
            .and_then(|o| serde_json::from_slice(String::from_utf8_lossy(&o.stdout).trim().as_bytes()).ok())
            .unwrap_or_default();
        for (i, file) in chunk.iter().enumerate() {
            let gone = std::fs::symlink_metadata(file).is_err();
            out.push(match parsed.get(i).cloned().flatten() {
                Some(p) if gone => Ok(Some(PathBuf::from(p)).filter(|p| !p.as_os_str().is_empty())),
                _ => Err(failed()),
            });
        }
    }
    out
}

#[cfg(target_os = "windows")]
fn os_trash(files: &[PathBuf]) -> Vec<Result<Option<PathBuf>, String>> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SCRIPT: &str = "Add-Type -AssemblyName Microsoft.VisualBasic;\
[Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($env:SMASH_TRASH,'OnlyErrorDialogs','SendToRecycleBin')";
    files
        .iter()
        .map(|file| {
            std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
                .env("SMASH_TRASH", file)
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(|e| e.to_string())?;
            if file.exists() {
                return Err(failed());
            }
            Ok(None)
        })
        .collect()
}

/// freedesktop 휴지통 (~/.local/share/Trash)
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn os_trash(files: &[PathBuf]) -> Vec<Result<Option<PathBuf>, String>> {
    files.iter().map(|f| freedesktop_trash(f)).collect()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn freedesktop_trash(file: &Path) -> Result<Option<PathBuf>, String> {
    use std::fs;
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| crate::config::home().join(".local").join("share"))
        .join("Trash");
    let files = base.join("files");
    let info = base.join("info");
    fs::create_dir_all(&files).map_err(|e| e.to_string())?;
    fs::create_dir_all(&info).map_err(|e| e.to_string())?;
    let name0 = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut name = name0.clone();
    let mut i = 2;
    while files.join(&name).exists() {
        let p = Path::new(&name0);
        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        name = match p.extension() {
            Some(ext) => format!("{stem}.{i}.{}", ext.to_string_lossy()),
            None => format!("{stem}.{i}"),
        };
        i += 1;
    }
    let date = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
    let encoded: String = file
        .to_string_lossy()
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    fs::write(info.join(format!("{name}.trashinfo")), format!("[Trash Info]\nPath={encoded}\nDeletionDate={date}\n"))
        .map_err(|e| e.to_string())?;
    let dest = files.join(&name);
    fs::rename(file, &dest).map_err(|e| e.to_string())?;
    Ok(Some(dest))
}

/// 여러 파일을 휴지통으로 보낸다. 결과는 넘긴 순서대로이고, 성공하면 휴지통 안 경로(알 수 있을 때)를 돌려준다.
pub fn move_many_to_trash(files: &[PathBuf]) -> Vec<Result<Option<PathBuf>, String>> {
    if files.is_empty() {
        return vec![];
    }
    match override_dir() {
        Some(dir) => files.iter().map(|f| trash_into(&dir, f)).collect(),
        None => os_trash(files),
    }
}

pub fn move_to_trash(file: &Path) -> Result<Option<PathBuf>, String> {
    move_many_to_trash(&[file.to_path_buf()]).pop().unwrap_or_else(|| Err(failed()))
}

/// 휴지통에 있는 파일을 원래 자리로 옮긴다. 원래 자리에 같은 이름이 생겼으면 덮어쓰지 않고 실패한다.
pub fn restore_from_trash(trashed: &Path, original: &Path) -> Result<(), String> {
    if std::fs::symlink_metadata(trashed).is_err() {
        return Err(l("휴지통에 그 파일이 없습니다(비웠거나 옮겼습니다)", "The file is no longer in the Trash"));
    }
    crate::fsops::move_file(trashed, original).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => l("원래 자리에 같은 이름 파일이 있습니다", "A file with the same name is in the original place"),
        std::io::ErrorKind::PermissionDenied => l("휴지통 폴더에 접근할 수 없습니다. 휴지통에서 직접 꺼내 주세요", "Can't access the Trash folder. Take it out of the Trash yourself"),
        _ => e.to_string(),
    })?;
    // freedesktop 휴지통은 정보 파일이 짝으로 있다
    if let (Some(dir), Some(name)) = (trashed.parent().and_then(Path::parent), trashed.file_name()) {
        let info = dir.join("info").join(format!("{}.trashinfo", name.to_string_lossy()));
        if info.is_file() {
            let _ = std::fs::remove_file(info);
        }
    }
    Ok(())
}

/// 이 OS 에서 휴지통으로 보낸 파일을 앱이 되돌릴 수 있는지
pub fn can_restore() -> bool {
    !cfg!(target_os = "windows") || override_dir().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{sandbox, write, LOCK};

    #[test]
    fn trash_and_restore_in_sandbox() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sb = sandbox();
        let dir = sb.join("trash-unit");
        let _ = std::fs::remove_dir_all(&dir);
        let files: Vec<PathBuf> = (0..BATCH + 5).map(|i| dir.join(format!("f{i}.txt"))).collect();
        for f in &files {
            write(f, "x", 0);
        }
        write(&dir.join("other").join("f0.txt"), "same name", 0);
        let res = move_many_to_trash(&files);
        assert_eq!(res.len(), files.len());
        let first = res[0].clone().unwrap().expect("휴지통 안 위치를 돌려준다");
        assert!(first.starts_with(sb.join("trash")), "시험은 임시 휴지통만 쓴다: {first:?}");
        assert!(files.iter().all(|f| !f.exists()));
        // 같은 이름을 또 버려도 먼저 버린 것을 덮어쓰지 않는다
        let second = move_to_trash(&dir.join("other").join("f0.txt")).unwrap().unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "x");
        // 되돌리기: 원래 자리에 같은 이름이 있으면 실패, 없으면 돌아온다
        write(&files[0], "blocker", 0);
        assert!(restore_from_trash(&first, &files[0]).is_err());
        std::fs::remove_file(&files[0]).unwrap();
        restore_from_trash(&first, &files[0]).unwrap();
        assert_eq!(std::fs::read_to_string(&files[0]).unwrap(), "x");
        assert!(restore_from_trash(&first, &files[0]).is_err(), "휴지통에 없으면 실패");
        assert!(move_to_trash(&dir.join("missing.txt")).is_err());
    }
}
