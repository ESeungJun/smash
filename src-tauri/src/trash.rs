//! OS 휴지통으로 보내기. 영구 삭제는 하지 않는다.

use std::path::Path;

use crate::lang::l;

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn failed() -> String {
    l("휴지통으로 옮기지 못했습니다", "Could not move to Trash")
}

/// NSFileManager 를 직접 불러서 Finder 자동화 권한 없이 휴지통으로 보낸다 (Electron 버전과 같은 방식)
#[cfg(target_os = "macos")]
pub fn move_to_trash(file: &Path) -> Result<(), String> {
    const SCRIPT: &str = "function run(argv){ObjC.import(\"Foundation\");\
return $.NSFileManager.defaultManager.trashItemAtURLResultingItemURLError($.NSURL.fileURLWithPath(argv[0]),null,null)}";
    let out = std::process::Command::new("osascript")
        .args(["-l", "JavaScript", "-e", SCRIPT])
        .arg(file)
        .output()
        .map_err(|e| e.to_string())?;
    if String::from_utf8_lossy(&out.stdout).trim() != "true" || file.exists() {
        return Err(failed());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn move_to_trash(file: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SCRIPT: &str = "Add-Type -AssemblyName Microsoft.VisualBasic;\
[Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($env:SMASH_TRASH,'OnlyErrorDialogs','SendToRecycleBin')";
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("SMASH_TRASH", file)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    if file.exists() {
        return Err(failed());
    }
    Ok(())
}

/// freedesktop 휴지통 (~/.local/share/Trash)
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn move_to_trash(file: &Path) -> Result<(), String> {
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
    fs::rename(file, files.join(&name)).map_err(|e| e.to_string())
}
