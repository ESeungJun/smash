//! 대분류(규칙의 목적지 폴더) 안의 소분류를 정한다.
//! 파일 내용은 보지 않고 이름·수정 시간·이미 있는 하위 폴더만 본다. 이미지 내용으로 묶기는 claude.rs 가 따로 한다.
//!
//! "auto" 순서
//!  1. 목적지에 이미 있는 하위 폴더 이름이 파일 이름에 들어 있으면 그 폴더 (사용자가 만든 분류를 먼저 따른다)
//!  2. 용도 단어: 영수증·계약서·스크린샷·카카오톡 …
//!  3. 카메라 사진·영상(IMG_, PXL_, 20261101_123456 …): 찍은 시간 묶음 → 카메라/2026-11-01
//!  4. 이름 앞부분이 같은 파일이 3개 이상이면 그 이름으로 (예: "Elden Ring 2026-…" → Elden Ring)
//!  5. 남은 사진·영상은 몇 시간 안에 몰려 있으면(4개 이상) 그날 날짜로
//!  6. 나머지는 대분류 폴더에 그대로

use std::collections::HashMap;

use chrono::TimeZone;

use crate::lang::l;
use crate::organizer::{ext_of, nfc};

/// 이름 앞부분이 같아도 묶지 않는 흔한 단어
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "new", "untitled", "image", "img", "file", "files", "document", "doc", "download", "copy", "final", "draft", "photo",
    "video", "screen", "제목", "제목없음", "제목 없음", "새", "문서", "사본", "복사본", "이미지", "사진", "파일", "최종",
];
const MIN_CLUSTER: usize = 3;
/// 이름 비교에 쓰는 기존 하위 폴더 수 상한(폴더가 아주 많아도 느려지지 않게)
const MAX_KNOWN: usize = 200;
const MIN_TIME_CLUSTER: usize = 4;
const EVENT_GAP_MS: f64 = 2.0 * 60.0 * 60.0 * 1000.0;
const MEDIA_EXTS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "heic", "heif", "webp", "tiff", "bmp", "raw", "dng", "cr2", "nef", "arw", "mp4", "mov", "avi", "mkv", "webm", "m4v",
];

struct Purpose {
    ko: &'static str,
    en: &'static str,
    words: &'static [&'static str],
}

/// 용도 단어. 위에서부터 처음 맞는 것. 단어는 소문자·NFC 로 비교한다.
const PURPOSES: &[Purpose] = &[
    Purpose { ko: "화면 녹화", en: "Screen recordings", words: &["화면 기록", "화면 녹화", "screen recording", "screenrecording", "녹화"] },
    Purpose { ko: "스크린샷", en: "Screenshots", words: &["스크린샷", "screenshot", "screen shot", "화면 캡처", "캡처", "capture", "snip"] },
    Purpose { ko: "카카오톡", en: "KakaoTalk", words: &["kakaotalk", "카카오톡", "카톡"] },
    Purpose { ko: "영수증·청구서", en: "Receipts & invoices", words: &["영수증", "receipt", "invoice", "청구서", "거래명세", "결제내역", "billing"] },
    Purpose { ko: "계약서", en: "Contracts", words: &["계약", "contract", "agreement", "동의서", "약정"] },
    Purpose { ko: "세금·증명서", en: "Tax & certificates", words: &["세금", "원천징수", "연말정산", "부가세", "증명서", "certificate", "등본", "초본", "확인서", "tax"] },
    Purpose { ko: "이력서·포트폴리오", en: "Resumes & portfolios", words: &["이력서", "resume", "résumé", "포트폴리오", "portfolio", "자기소개서", "경력기술서", "자소서"] },
    Purpose { ko: "견적·제안서", en: "Quotes & proposals", words: &["견적", "quotation", "estimate", "제안서", "proposal"] },
    Purpose { ko: "회의·보고서", en: "Meetings & reports", words: &["회의", "meeting", "minutes", "보고서", "report"] },
    Purpose { ko: "공부·강의", en: "Study", words: &["강의", "lecture", "과제", "homework", "assignment", "시험", "exam", "수업", "lesson", "syllabus", "논문", "thesis"] },
    Purpose { ko: "여행", en: "Travel", words: &["여행", "travel", "itinerary", "탑승권", "boarding", "항공권", "e-ticket", "호텔", "hotel", "booking"] },
    Purpose { ko: "설명서", en: "Manuals", words: &["manual", "설명서", "사용법", "user guide", "가이드북"] },
    Purpose { ko: "배경화면", en: "Wallpapers", words: &["wallpaper", "배경화면"] },
    Purpose { ko: "로고·아이콘", en: "Logos & icons", words: &["logo", "로고", "icon", "아이콘", "favicon"] },
    Purpose { ko: "목업·시안", en: "Mockups", words: &["mockup", "목업", "시안", "wireframe", "와이어프레임"] },
    Purpose { ko: "백업", en: "Backups", words: &["backup", "백업"] },
];

/// 소분류할 파일 하나
pub struct GFile {
    pub name: String,
    pub mtime_ms: f64,
}

fn lower(s: &str) -> String {
    nfc(s).to_lowercase()
}

fn stem(name: &str) -> String {
    std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string())
}

fn is_media(name: &str) -> bool {
    MEDIA_EXTS.contains(&ext_of(name).as_str())
}

/// 폰·카메라가 붙이는 이름: IMG_1234, DSC01234, PXL_20261101_…, 20261101_123456, VID_…, MVIMG_…
fn is_camera_name(name: &str) -> bool {
    let s = lower(&stem(name));
    let prefixed = ["img_", "img-", "dsc", "dscn", "dcim", "pxl_", "vid_", "mvimg_", "gopr", "dji_", "p_20", "photo_20"].iter().any(|p| s.starts_with(p))
        && s.chars().any(|c| c.is_ascii_digit());
    let digits: String = s.chars().take(15).collect();
    let dated = digits.is_ascii() && digits.len() == 15 && digits[..8].chars().all(|c| c.is_ascii_digit()) && matches!(&digits[8..9], "_" | "-") && digits[9..].chars().all(|c| c.is_ascii_digit());
    prefixed || dated
}

/// 이름에 그 낱말이 들어 있는지. 영문·숫자 3글자 이하는 앞뒤가 글자가 아닐 때만(낱말 경계).
fn name_has(name: &str, word: &str) -> bool {
    if !(word.is_ascii() && word.len() <= 3) {
        return name.contains(word);
    }
    name.match_indices(word).any(|(i, _)| {
        let before = name[..i].chars().next_back();
        let after = name[i + word.len()..].chars().next();
        !before.map(|c| c.is_alphanumeric()).unwrap_or(false) && !after.map(|c| c.is_alphanumeric()).unwrap_or(false)
    })
}

fn purpose_of(name: &str) -> Option<&'static Purpose> {
    let n = lower(name);
    PURPOSES.iter().find(|p| p.words.iter().any(|w| n.contains(w)))
}

fn is_noise_token(t: &str) -> bool {
    t.is_empty()
        || t.chars().all(|c| c.is_ascii_digit() || c == '.')
        || STOP_WORDS.contains(&t)
        || (t.chars().count() < 2)
        || (t.is_ascii() && t.chars().count() < 3)
}

/// 이름 앞쪽의 의미 있는 낱말들 (원래 대소문자 유지). 숫자·날짜·복사 표시는 뺀다.
fn lead_tokens(name: &str) -> Vec<String> {
    let s = nfc(&stem(name));
    s.split([' ', '_', '-', '.', ',', '(', ')', '[', ']'])
        .map(str::trim)
        .take_while(|t| !t.chars().all(|c| c.is_ascii_digit()) || t.is_empty())
        .filter(|t| !t.is_empty())
        .filter(|t| !is_noise_token(&t.to_lowercase()))
        .take(2)
        .map(String::from)
        .collect()
}

/// 윈도우가 폴더 이름으로 받지 않는 이름(CON, NUL, COM1 …)
fn is_reserved_name(p: &str) -> bool {
    let stem = p.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit())
}

/// 폴더 이름으로 쓸 수 있게 다듬는다. 두 단계(a/b)까지 허용.
pub fn clean_folder(raw: &str) -> Option<String> {
    let parts: Vec<String> = raw
        .split(['/', '\\'])
        .map(|p| {
            // 금지 문자, 제어 문자, 보이지 않는 서식 문자(폭 없는 공백·방향 바꾸기)를 뺀다
            let cleaned: String = p
                .chars()
                .filter(|c| !matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control())
                .filter(|c| !matches!(*c as u32, 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x2069 | 0xFEFF | 0x00AD))
                .collect();
            let t = cleaned.trim().trim_matches('.').trim();
            // 자른 뒤 끝에 공백·점이 남으면 윈도우가 폴더를 못 만든다
            t.chars().take(40).collect::<String>().trim_end_matches(['.', ' ']).to_string()
        })
        .filter(|p| !p.is_empty() && p != ".." && !is_reserved_name(p))
        .take(2)
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn local_date(ms: f64) -> String {
    chrono::Local.timestamp_millis_opt(ms as i64).single().map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

fn local_month(ms: f64) -> String {
    chrono::Local.timestamp_millis_opt(ms as i64).single().map(|d| d.format("%Y-%m").to_string()).unwrap_or_default()
}

/// 시간순으로 몰린 묶음들 (간격이 EVENT_GAP_MS 보다 크면 끊는다). 묶음마다 파일 번호 목록.
fn time_clusters(files: &[GFile], idx: &[usize]) -> Vec<Vec<usize>> {
    let mut sorted: Vec<usize> = idx.to_vec();
    sorted.sort_by(|a, b| files[*a].mtime_ms.partial_cmp(&files[*b].mtime_ms).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for i in sorted {
        let t = files[i].mtime_ms;
        if out.is_empty() || t - last > EVENT_GAP_MS {
            out.push(vec![]);
        }
        out.last_mut().unwrap().push(i);
        last = t;
    }
    out
}

/// 파일마다 대분류 폴더 안 소분류(없으면 None). `existing` 은 목적지에 이미 있는 하위 폴더 이름들.
pub fn subfolders(mode: &str, files: &[GFile], existing: &[String]) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = vec![None; files.len()];
    match mode {
        "month" => {
            for (i, f) in files.iter().enumerate() {
                let m = local_month(f.mtime_ms);
                out[i] = if m.is_empty() { None } else { Some(m) };
            }
            return out;
        }
        "auto" => {}
        _ => return out,
    }

    // 1. 이미 있는 하위 폴더 (긴 이름 먼저: "엘든 링 DLC" 가 "엘든 링" 보다 먼저)
    //    날짜 모양 폴더(2026-11 등)는 이름 비교에 쓰지 않는다. 영문 3글자 이하는 낱말로 맞아야 한다(go 가 logo 를 끌어가지 않게).
    let mut known: Vec<(String, String)> = existing
        .iter()
        .map(|e| (lower(e), e.clone()))
        .filter(|(k, _)| k.chars().count() >= 2 && !k.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | '_' | '.' | ' ')))
        .collect();
    known.sort_by_key(|(k, _)| std::cmp::Reverse(k.chars().count()));
    known.truncate(MAX_KNOWN);
    let stems: Vec<String> = files.iter().map(|f| lower(&stem(&f.name))).collect();
    for (i, n) in stems.iter().enumerate() {
        if let Some((_, orig)) = known.iter().find(|(k, _)| name_has(n, k)) {
            out[i] = Some(orig.clone());
        }
    }

    // 2. 용도 단어
    for (i, f) in files.iter().enumerate() {
        if out[i].is_none() {
            if let Some(p) = purpose_of(&f.name) {
                out[i] = Some(l(p.ko, p.en));
            }
        }
    }

    // 3. 카메라 사진·영상 → 카메라/날짜 (그날 몰린 게 적으면 카메라에만)
    let camera = l("카메라", "Camera");
    let cam_idx: Vec<usize> = (0..files.len()).filter(|&i| out[i].is_none() && is_camera_name(&files[i].name)).collect();
    for cluster in time_clusters(files, &cam_idx) {
        let label = if cluster.len() >= MIN_TIME_CLUSTER { format!("{camera}/{}", local_date(files[cluster[0]].mtime_ms)) } else { camera.clone() };
        for i in cluster {
            out[i] = Some(label.clone());
        }
    }

    // 4. 이름 앞부분이 같은 파일 묶음: 두 낱말 → 한 낱말 순서로
    for width in [2usize, 1] {
        let mut by_key: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, f) in files.iter().enumerate() {
            if out[i].is_some() {
                continue;
            }
            let tokens = lead_tokens(&f.name);
            if tokens.len() < width {
                continue;
            }
            by_key.entry(tokens[..width].join(" ").to_lowercase()).or_default().push(i);
        }
        let mut keys: Vec<_> = by_key.into_iter().filter(|(_, v)| v.len() >= MIN_CLUSTER).collect();
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, members) in keys {
            let label = lead_tokens(&files[members[0]].name)[..width].join(" ");
            let Some(label) = clean_folder(&label) else { continue };
            for i in members {
                out[i] = Some(label.clone());
            }
        }
    }

    // 5. 남은 사진·영상: 몇 시간 안에 몰린 묶음은 그날 날짜로
    let media_idx: Vec<usize> = (0..files.len()).filter(|&i| out[i].is_none() && is_media(&files[i].name)).collect();
    for cluster in time_clusters(files, &media_idx) {
        if cluster.len() >= MIN_TIME_CLUSTER {
            let label = local_date(files[cluster[0]].mtime_ms);
            for i in cluster {
                out[i] = Some(label.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::set_lang;
    use crate::test_util::LOCK;
    use unicode_normalization::UnicodeNormalization;

    fn f(name: &str, mtime_ms: f64) -> GFile {
        GFile { name: name.into(), mtime_ms }
    }

    const H: f64 = 60.0 * 60.0 * 1000.0;

    #[test]
    fn auto_groups_by_folder_purpose_camera_prefix_time() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_lang("ko");
        let base = 1_793_000_000_000.0;
        let shot: String = "스크린샷 2026-11-01 오후 9.20.11.png".nfd().collect();
        let files = vec![
            f(&shot, base),                              // 0 용도: 스크린샷
            f("KakaoTalk_20261101_123456789.jpg", base), // 1 용도: 카카오톡
            f("엘든링_보스.png", base),                  // 2 이미 있는 폴더
            f("IMG_0001.HEIC", base),                    // 3~6 카메라, 한 시간 안
            f("IMG_0002.HEIC", base + 0.2 * H),
            f("IMG_0003.HEIC", base + 0.4 * H),
            f("IMG_0004.HEIC", base + 0.6 * H),
            f("IMG_0100.HEIC", base + 100.0 * H),        // 7 카메라, 혼자
            f("Elden Ring 2026-11-01 1.png", base),      // 8~10 이름 앞부분
            f("Elden Ring 2026-11-01 2.png", base),
            f("Elden Ring 2026-11-02 3.png", base),
            f("meme-a.png", base + 300.0 * H),           // 11~14 남은 사진, 몰려 있음
            f("cat.png", base + 300.1 * H),
            f("dog.png", base + 300.2 * H),
            f("tree.png", base + 300.3 * H),
            f("lonely.png", base + 500.0 * H),           // 15 그대로
            f("2026 연봉 계약서.pdf", base),             // 16 용도: 계약서
        ];
        let got = subfolders("auto", &files, &["엘든링".into(), "go".into(), "2026-11".into()]);
        let s = |i: usize| got[i].clone().unwrap_or_default();
        assert_eq!(s(0), "스크린샷");
        assert_eq!(s(1), "카카오톡");
        assert_eq!(s(2), "엘든링");
        assert!(s(3).starts_with("카메라/20"), "{}", s(3));
        assert_eq!(s(3), s(6));
        assert_eq!(s(7), "카메라");
        assert_eq!(s(8), "Elden Ring");
        assert_eq!(s(10), "Elden Ring");
        assert!(s(11).starts_with("20") && s(11) == s(14), "{}", s(11));
        assert_eq!(got[15], None);
        assert_eq!(s(16), "계약서");
        let got = subfolders("auto", &[f("logo.png", base), f("go 대회.png", base)], &["go".into()]);
        assert_eq!(got[0].as_deref(), Some("로고·아이콘"), "go 폴더가 logo 를 끌어가지 않는다");
        assert_eq!(got[1].as_deref(), Some("go"));
    }

    #[test]
    fn none_and_month_modes() {
        let files = vec![f("a.png", 1_793_000_000_000.0)];
        assert_eq!(subfolders("", &files, &[]), vec![None]);
        assert!(subfolders("month", &files, &[])[0].as_deref().unwrap().starts_with("2026-"));
    }

    #[test]
    fn small_prefix_groups_stay_loose() {
        let files = vec![f("report-final.pdf", 0.0), f("report-v2.pdf", 0.0), f("untitled 1.txt", 0.0), f("untitled 2.txt", 0.0), f("untitled 3.txt", 0.0)];
        let got = subfolders("auto", &files, &[]);
        assert_eq!(got[0].as_deref(), Some(l("회의·보고서", "Meetings & reports").as_str()));
        assert!(got[2].is_none(), "흔한 단어(untitled)로는 묶지 않는다");
    }

    #[test]
    fn folder_names_are_cleaned() {
        assert_eq!(clean_folder("게임/엘든 링: DLC?"), Some("게임/엘든 링 DLC".into()));
        assert_eq!(clean_folder("../../etc"), Some("etc".into()), "상위 폴더로 못 나간다");
        assert_eq!(clean_folder("a/b/c"), Some("a/b".into()));
        assert_eq!(clean_folder("  "), None);
        assert_eq!(clean_folder("게임\u{202E}gnp/NUL"), Some("게임gnp".into()), "보이지 않는 문자·예약 이름은 뺀다");
    }
}
