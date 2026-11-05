//! 이미지·동영상 썸네일(긴 변 384px JPEG)을 임시 폴더에 만든다. '이미지 내용으로 묶기'를 켰을 때만 쓴다.
//!  - macOS: 이미지는 `sips`(HEIC 포함), 동영상은 `qlmanage` 로 한 장 뜬 뒤 `sips` 로 줄인다
//!  - Windows: 탐색기와 같은 셸 썸네일(IShellItemImageFactory)을 PowerShell 로 저장한다 (이미지·동영상 모두)
//!  - 그 밖: 지원하지 않는다

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub const THUMB_SIZE: u32 = 384;
pub const TMP_PREFIX: &str = "smash-thumbs-";
/// 손상된 동영상이나 다른 앱의 썸네일 처리기가 멈춰도 끝나도록
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const PER_FILE_TIMEOUT: Duration = Duration::from_secs(20);
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const BATCH_TIMEOUT: Duration = Duration::from_secs(180);

/// 명령을 띄우고 제한 시간 안에 성공으로 끝났는지. 넘으면 끈다.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn run_with_timeout(cmd: &mut Command, limit: Duration) -> bool {
    use std::process::Stdio;
    let Ok(mut child) = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return false };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return st.success(),
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// 앱이 도중에 꺼져 남은 예전 썸네일 폴더(한 시간 넘은 것)를 지운다.
pub fn clean_stale_dirs() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|d| d > Duration::from_secs(3600)).unwrap_or(false);
        if name.starts_with(TMP_PREFIX) && old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

pub const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "tiff", "tif", "bmp"];
pub const VIDEO_EXTS: &[&str] = &["mp4", "mov", "m4v", "avi", "mkv", "webm"];

pub fn supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

pub fn is_thumbable(name: &str) -> bool {
    let ext = crate::organizer::ext_of(name);
    IMAGE_EXTS.contains(&ext.as_str()) || VIDEO_EXTS.contains(&ext.as_str())
}

/// srcs[i] 의 썸네일을 out_dir/i.jpg 로 만든다. 실패한 자리는 None.
pub fn make_thumbs(srcs: &[PathBuf], out_dir: &Path) -> Vec<Option<PathBuf>> {
    if std::fs::create_dir_all(out_dir).is_err() {
        return vec![None; srcs.len()];
    }
    platform_thumbs(srcs, out_dir)
        .into_iter()
        .map(|p| p.filter(|p| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)))
        .collect()
}

#[cfg(target_os = "macos")]
fn mac_thumb(i: usize, src: &Path, out_dir: &Path) -> Option<PathBuf> {
    let size = THUMB_SIZE.to_string();
    // 경로는 항상 절대 경로라 '-' 로 시작해 옵션으로 읽히는 일이 없다
    let sips = |src: &Path, dst: &Path| {
        src.is_absolute()
            && run_with_timeout(
                Command::new("/usr/bin/sips").args(["-s", "format", "jpeg", "-s", "formatOptions", "70", "-Z", &size]).arg(src).arg("--out").arg(dst),
                PER_FILE_TIMEOUT,
            )
    };
    let dst = out_dir.join(format!("{i}.jpg"));
    let ext = crate::organizer::ext_of(&src.to_string_lossy());
    if VIDEO_EXTS.contains(&ext.as_str()) {
        // qlmanage 는 <out>/<파일 이름>.png 로 쓴다. 이름이 겹치지 않게 파일마다 폴더를 따로 쓴다.
        let ql_dir = out_dir.join(format!("ql{i}"));
        let _ = std::fs::create_dir_all(&ql_dir);
        let ok = src.is_absolute() && run_with_timeout(Command::new("/usr/bin/qlmanage").args(["-t", "-s", &size, "-o"]).arg(&ql_dir).arg(src), PER_FILE_TIMEOUT);
        let png = std::fs::read_dir(&ql_dir).ok().and_then(|mut d| d.find_map(|e| e.ok().map(|e| e.path())));
        let made = ok && png.map(|p| sips(&p, &dst)).unwrap_or(false);
        let _ = std::fs::remove_dir_all(&ql_dir);
        made.then_some(dst)
    } else {
        sips(src, &dst).then_some(dst)
    }
}

/// 파일 하나에 프로세스 하나라 네 개씩 나눠 동시에 만든다.
#[cfg(target_os = "macos")]
fn platform_thumbs(srcs: &[PathBuf], out_dir: &Path) -> Vec<Option<PathBuf>> {
    const WORKERS: usize = 4;
    let mut out: Vec<Option<PathBuf>> = vec![None; srcs.len()];
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..WORKERS)
            .map(|w| {
                scope.spawn(move || {
                    (w..srcs.len()).step_by(WORKERS).map(|i| (i, mac_thumb(i, &srcs[i], out_dir))).collect::<Vec<_>>()
                })
            })
            .collect();
        for h in handles {
            for (i, r) in h.join().unwrap_or_default() {
                out[i] = r;
            }
        }
    });
    out
}

#[cfg(target_os = "windows")]
const PS_SCRIPT: &str = r#"
param([string]$ListPath)
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Drawing;
using System.Drawing.Imaging;
public static class SmashThumb {
  [ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  public interface IShellItemImageFactory { void GetImage(Size2 size, int flags, out IntPtr phbm); }
  [StructLayout(LayoutKind.Sequential)]
  public struct Size2 { public int cx; public int cy; }
  [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
  static extern void SHCreateItemFromParsingName(string path, IntPtr pbc, [MarshalAs(UnmanagedType.LPStruct)] Guid riid, out IShellItemImageFactory ppv);
  [DllImport("gdi32.dll")]
  static extern bool DeleteObject(IntPtr h);
  public static void Save(string src, string dst, int size) {
    IShellItemImageFactory f;
    SHCreateItemFromParsingName(src, IntPtr.Zero, new Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), out f);
    IntPtr hbm;
    Size2 s; s.cx = size; s.cy = size;
    f.GetImage(s, 0, out hbm);
    try { using (Bitmap bmp = Image.FromHbitmap(hbm)) { bmp.Save(dst, ImageFormat.Jpeg); } }
    finally { DeleteObject(hbm); }
  }
}
"@
$items = Get-Content -Raw -Encoding UTF8 -LiteralPath $ListPath | ConvertFrom-Json
foreach ($i in $items) { try { [SmashThumb]::Save($i.src, $i.dst, [int]$i.size) } catch { } }
"#;

#[cfg(target_os = "windows")]
fn platform_thumbs(srcs: &[PathBuf], out_dir: &Path) -> Vec<Option<PathBuf>> {
    use std::os::windows::process::CommandExt;
    let dsts: Vec<PathBuf> = (0..srcs.len()).map(|i| out_dir.join(format!("{i}.jpg"))).collect();
    // 네트워크 경로(\\서버\공유)는 셸이 그 서버에 접속하므로 빼 둔다
    let list: Vec<serde_json::Value> = srcs
        .iter()
        .zip(&dsts)
        .filter(|(s, _)| s.is_absolute() && !s.to_string_lossy().starts_with("\\\\"))
        .map(|(s, d)| serde_json::json!({ "src": s.to_string_lossy(), "dst": d.to_string_lossy(), "size": THUMB_SIZE }))
        .collect();
    let script = out_dir.join("thumbs.ps1");
    let list_path = out_dir.join("list.json");
    // PowerShell 5 는 BOM 이 있어야 UTF-8 스크립트로 읽는다
    let wrote = std::fs::write(&script, format!("\u{feff}{PS_SCRIPT}")).is_ok() && std::fs::write(&list_path, serde_json::Value::Array(list).to_string()).is_ok();
    if wrote {
        run_with_timeout(
            Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(&script)
                .arg(&list_path)
                .creation_flags(0x0800_0000),
            BATCH_TIMEOUT,
        );
    }
    dsts.into_iter().map(|d| d.exists().then_some(d)).collect()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn platform_thumbs(srcs: &[PathBuf], _out_dir: &Path) -> Vec<Option<PathBuf>> {
    vec![None; srcs.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbable_kinds() {
        assert!(is_thumbable("a.HEIC"));
        assert!(is_thumbable("clip.mov"));
        assert!(!is_thumbable("doc.pdf"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn makes_jpeg_thumbnail_on_mac() {
        let dir = crate::test_util::sandbox().join("thumbs");
        let _ = std::fs::remove_dir_all(&dir);
        let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("assets").join("icon-256.png");
        if !icon.exists() {
            return;
        }
        let got = make_thumbs(&[icon, dir.join("없음.png")], &dir);
        assert!(got[0].is_some());
        assert!(got[1].is_none());
    }
}
