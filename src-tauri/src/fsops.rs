//! 파일 시스템 쪽 낮은 층: 덮어쓰지 않는 이동, 다른 디스크로 복사, 받은 시각, 받은 곳.
//! macOS 전용 호출(libc)만 여기 모으고, 다른 OS 는 표준 라이브러리로만 한다.

use std::fs;
use std::io;
use std::path::Path;

use crate::config::new_id;

pub fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV(18, 유닉스) · ERROR_NOT_SAME_DEVICE(17, 윈도우)
    matches!(e.raw_os_error(), Some(18) if cfg!(unix)) || matches!(e.raw_os_error(), Some(17) if cfg!(windows))
}

fn exists_err() -> io::Error {
    io::Error::from(io::ErrorKind::AlreadyExists)
}

/// OS 가 "없을 때만 이름 바꾸기"를 해 주면 그걸 쓴다. 지원하지 않으면 None.
#[cfg(target_os = "macos")]
fn rename_excl(from: &Path, to: &Path) -> Option<io::Result<()>> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let (Ok(f), Ok(t)) = (CString::new(from.as_os_str().as_bytes()), CString::new(to.as_os_str().as_bytes())) else {
        return Some(Err(io::Error::from(io::ErrorKind::InvalidInput)));
    };
    // SAFETY: 둘 다 NUL 로 끝나는 살아 있는 C 문자열이다
    let rc = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
    if rc == 0 {
        return Some(Ok(()));
    }
    let e = io::Error::last_os_error();
    match e.raw_os_error() {
        // 이 파일 시스템은 RENAME_EXCL 을 모른다 (예: 일부 네트워크·FAT 볼륨)
        Some(libc::ENOTSUP) | Some(libc::EINVAL) => None,
        _ => Some(Err(e)),
    }
}

#[cfg(not(target_os = "macos"))]
fn rename_excl(_from: &Path, _to: &Path) -> Option<io::Result<()>> {
    None
}

/// 목적지가 이미 있으면 덮어쓰지 않고 `AlreadyExists` 로 실패한다.
/// 확인과 이동 사이에 다른 프로그램이 같은 이름을 만들어도 덮어쓰지 않는다.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(res) = rename_excl(from, to) {
        return res;
    }
    // 하드 링크는 목적지가 있으면 실패한다. 링크를 건 뒤 원래 이름을 지우면 덮어쓰지 않는 이동이 된다.
    match fs::hard_link(from, to) {
        Ok(()) => fs::remove_file(from).inspect_err(|_| {
            let _ = fs::remove_file(to);
        }),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists || is_cross_device(&e) => Err(e),
        Err(_) => {
            // 하드 링크를 못 쓰는 파일 시스템(FAT 등): 확인한 뒤 옮긴다. 여기서는 틈이 남는다.
            if fs::symlink_metadata(to).is_ok() {
                return Err(exists_err());
            }
            fs::rename(from, to)
        }
    }
}

/// 다른 디스크로: 목적지 폴더에 임시 이름으로 복사(권한·확장 속성 포함)해 디스크에 내려쓴 뒤
/// 덮어쓰지 않는 이름 바꾸기로 제 이름을 주고, 그다음 원본을 지운다. 실패하면 임시 파일을 지운다.
pub fn copy_then_remove(from: &Path, to: &Path) -> io::Result<()> {
    let dir = to.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(".smash-{}.tmp", new_id("c")));
    let copied = (|| {
        let meta = fs::metadata(from)?;
        fs::copy(from, &tmp)?;
        let _ = filetime::set_file_times(&tmp, filetime::FileTime::from_last_access_time(&meta), filetime::FileTime::from_last_modification_time(&meta));
        fs::File::open(&tmp)?.sync_all()?;
        rename_noreplace(&tmp, to)
    })();
    if let Err(e) = copied {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // 원본을 못 지우면 같은 파일이 두 곳에 남는다. 복사본을 지워 옮기기 전 상태로 돌린다.
    fs::remove_file(from).inspect_err(|_| {
        let _ = fs::remove_file(to);
    })
}

/// 파일 하나를 옮긴다. 기존 파일은 덮어쓰지 않는다(`AlreadyExists`).
pub fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir)?;
    }
    match rename_noreplace(from, to) {
        Err(e) if is_cross_device(&e) => copy_then_remove(from, to),
        other => other,
    }
}

fn to_ms(t: std::time::SystemTime) -> Option<f64> {
    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs_f64() * 1000.0)
}

/// Finder 의 "추가된 날짜": 이 폴더에 들어온 시각. 볼륨이 지원하지 않으면 None.
#[cfg(target_os = "macos")]
fn added_ms(path: &Path) -> Option<f64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    #[repr(C, packed(4))]
    struct Buf {
        len: u32,
        returned: libc::attribute_set_t,
        added: libc::timespec,
    }
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: 0 으로 채운 구조체는 둘 다 유효한 값이다
    let mut list: libc::attrlist = unsafe { std::mem::zeroed() };
    list.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    list.commonattr = libc::ATTR_CMN_RETURNED_ATTRS | libc::ATTR_CMN_ADDEDTIME;
    let mut buf: Buf = unsafe { std::mem::zeroed() };
    // SAFETY: 경로는 NUL 로 끝나고, 버퍼 크기를 그대로 넘긴다
    let rc = unsafe {
        libc::getattrlist(
            c.as_ptr(),
            &mut list as *mut libc::attrlist as *mut libc::c_void,
            &mut buf as *mut Buf as *mut libc::c_void,
            std::mem::size_of::<Buf>(),
            libc::FSOPT_NOFOLLOW,
        )
    };
    let returned = buf.returned;
    if rc != 0 || returned.commonattr & libc::ATTR_CMN_ADDEDTIME == 0 {
        return None;
    }
    let ts = buf.added;
    if ts.tv_sec <= 0 {
        return None;
    }
    Some(ts.tv_sec as f64 * 1000.0 + ts.tv_nsec as f64 / 1.0e6)
}

#[cfg(not(target_os = "macos"))]
fn added_ms(_path: &Path) -> Option<f64> {
    None
}

/// 이 파일을 받은 시각(ms). macOS 는 폴더에 추가된 시각, 없거나 다른 OS 면 만든 시각.
/// 둘 다 모르면 None 이고, 그러면 "받은 지 N일" 조건은 맞지 않는 것으로 본다(옮기지 않는다).
pub fn received_ms(path: &Path, meta: &fs::Metadata) -> Option<f64> {
    #[cfg(test)]
    if let Some(v) = crate::test_util::received_override(path) {
        return v;
    }
    added_ms(path).or_else(|| meta.created().ok().and_then(to_ms))
}

/// 내려받은 주소 기록(원본 바이트). macOS 는 `kMDItemWhereFroms` 확장 속성, 윈도우는 `Zone.Identifier` 스트림.
#[cfg(target_os = "macos")]
pub fn where_from(path: &Path) -> Option<Vec<u8>> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let name = c"com.apple.metadata:kMDItemWhereFroms";
    let mut buf = vec![0u8; 16 * 1024];
    // SAFETY: 버퍼와 그 길이를 그대로 넘긴다
    let n = unsafe { libc::getxattr(c.as_ptr(), name.as_ptr(), buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0, libc::XATTR_NOFOLLOW) };
    if n <= 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(buf)
}

#[cfg(target_os = "windows")]
pub fn where_from(path: &Path) -> Option<Vec<u8>> {
    let mut stream = path.as_os_str().to_os_string();
    stream.push(":Zone.Identifier");
    fs::read(stream).ok().filter(|b| !b.is_empty())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn where_from(_path: &Path) -> Option<Vec<u8>> {
    None
}

/// 주소 기록 안에 낱말이 들어 있는지(대소문자 무시). 기록은 이진 plist 라 글자가 ASCII 또는 UTF-16BE 로 들어 있다.
pub fn from_contains(raw: &[u8], needle: &str) -> bool {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return false;
    }
    let hay: Vec<u8> = raw.to_ascii_lowercase();
    let find = |pat: &[u8]| !pat.is_empty() && hay.windows(pat.len()).any(|w| w == pat);
    // 건초더미를 바이트 단위로 소문자로 바꿨으니 UTF-16 패턴도 같은 변환을 거친다
    let utf16: Vec<u8> = needle.encode_utf16().flat_map(|u| u.to_be_bytes()).collect::<Vec<u8>>().to_ascii_lowercase();
    find(needle.as_bytes()) || (!needle.is_ascii() && find(&utf16))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{sandbox, write};

    #[test]
    fn rename_never_overwrites() {
        let dir = sandbox().join("fsops-rename");
        let _ = fs::remove_dir_all(&dir);
        write(&dir.join("a.txt"), "mine", 0);
        write(&dir.join("b.txt"), "theirs", 0);
        let err = move_file(&dir.join("a.txt"), &dir.join("b.txt")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "theirs");
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "mine", "실패하면 원본은 그 자리에");
        // 끊어진 심볼릭 링크도 "있는 것"으로 본다
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("dangling.txt")).unwrap();
            assert_eq!(move_file(&dir.join("a.txt"), &dir.join("dangling.txt")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        }
        move_file(&dir.join("a.txt"), &dir.join("sub").join("c.txt")).unwrap();
        assert_eq!(fs::read_to_string(dir.join("sub").join("c.txt")).unwrap(), "mine");
    }

    #[test]
    fn copy_keeps_attributes_and_cleans_up() {
        let dir = sandbox().join("fsops-copy");
        let _ = fs::remove_dir_all(&dir);
        write(&dir.join("tool.sh"), "#!/bin/sh\n", 3);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dir.join("tool.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(target_os = "macos")]
        let tagged = std::process::Command::new("xattr").args(["-w", "com.apple.metadata:kMDItemWhereFroms", "https://example.com/tool.sh"]).arg(dir.join("tool.sh")).status().map(|s| s.success()).unwrap_or(false);
        let before = fs::metadata(dir.join("tool.sh")).unwrap().modified().unwrap();
        fs::create_dir_all(dir.join("out")).unwrap();
        copy_then_remove(&dir.join("tool.sh"), &dir.join("out").join("tool.sh")).unwrap();
        let to = dir.join("out").join("tool.sh");
        assert!(!dir.join("tool.sh").exists());
        assert_eq!(fs::metadata(&to).unwrap().modified().unwrap(), before, "수정 시각 유지");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&to).unwrap().permissions().mode() & 0o777, 0o755, "실행 권한 유지");
        }
        #[cfg(target_os = "macos")]
        if tagged {
            assert!(from_contains(&where_from(&to).unwrap(), "EXAMPLE.com"), "확장 속성(받은 곳) 유지");
        }
        // 목적지가 이미 있으면 덮어쓰지 않고 임시 파일도 남기지 않는다
        write(&dir.join("again.sh"), "new", 0);
        write(&dir.join("out").join("again.sh"), "old", 0);
        assert_eq!(copy_then_remove(&dir.join("again.sh"), &dir.join("out").join("again.sh")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(dir.join("out").join("again.sh")).unwrap(), "old");
        assert!(dir.join("again.sh").exists());
        let leftovers: Vec<_> = fs::read_dir(dir.join("out")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".tmp")).collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn received_time_is_arrival_not_mtime() {
        let dir = sandbox().join("fsops-received");
        let _ = fs::remove_dir_all(&dir);
        let p = dir.join("old-date.pkg");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, "x").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(2000 * 86_400);
        let ft = filetime::FileTime::from_system_time(old);
        filetime::set_file_times(&p, ft, ft).unwrap();
        let meta = fs::metadata(&p).unwrap();
        // macOS: 추가된 시각은 수정 시각을 옛날로 바꿔도 지금이다. 다른 OS 는 만든 시각이 없으면 None(보수적으로 안 옮김).
        if let Some(got) = added_ms(&p).or_else(|| if cfg!(target_os = "macos") { None } else { meta.created().ok().and_then(to_ms) }) {
            assert!(crate::config::now_ms() - got < 60_000.0, "받은 시각은 방금이어야 한다: {got}");
        }
        #[cfg(target_os = "macos")]
        assert!(added_ms(&p).is_some(), "APFS 에서는 추가된 시각을 읽을 수 있다");
    }

    #[test]
    fn where_from_matching() {
        assert!(from_contains(b"bplist00\xa2_\x10\x1ehttps://GitHub.com/a/b/x.zip", "github.com"));
        assert!(!from_contains(b"https://example.com", "github.com"));
        assert!(!from_contains(b"anything", "  "));
        let utf16: Vec<u8> = "https://예시.kr/a".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert!(from_contains(&utf16, "예시"));
        // UTF-16 하위 바이트가 대문자 범위(0x41~0x5A)에 걸리는 글자도 맞아야 한다 ('학' U+D559)
        let hak: Vec<u8> = "https://대학.kr".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert!(from_contains(&hak, "대학"));
    }
}
