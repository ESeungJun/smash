//! 설정 파일 위치·기본 규칙·읽고 쓰기. 앱과 MCP 서버가 같은 파일을 쓴다.
//! Electron 버전과 같은 위치·같은 JSON 모양을 써서 저장된 규칙·기록이 그대로 이어진다.

use std::path::{Component, Path, PathBuf, MAIN_SEPARATOR};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize, Serializer};
use serde_json::{json, Value};

use crate::lang::l;

const APP_DIR_NAME: &str = "Smash";

// ── 숫자: 정수면 정수로 쓴다 (예전 파일과 같은 모양, 화면의 === 비교도 그대로) ──
pub fn ser_num<S: Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    if v.fract() == 0.0 && v.abs() < 9.0e15 {
        s.serialize_i64(*v as i64)
    } else {
        s.serialize_f64(*v)
    }
}

// ── 경로 ──────────────────────────────────────────────

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SMASH_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let home = home();
    if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support").join(APP_DIR_NAME)
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"))
            .join(APP_DIR_NAME)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("smash")
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn history_path() -> PathBuf {
    config_dir().join("history.json")
}

pub fn expand_home(p: &str) -> String {
    if p == "~" {
        return home().to_string_lossy().into_owned();
    }
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        return home().join(rest).to_string_lossy().into_owned();
    }
    p.to_string()
}

pub fn contract_home(p: &str) -> String {
    let home = home().to_string_lossy().into_owned();
    if p == home {
        return "~".into();
    }
    let prefix = format!("{home}{MAIN_SEPARATOR}");
    if let Some(rest) = p.strip_prefix(&prefix) {
        return format!("~/{}", rest.split(MAIN_SEPARATOR).collect::<Vec<_>>().join("/"));
    }
    p.to_string()
}

/// Node 의 path.resolve 처럼: 상대 경로면 base 에 붙이고, `.`·`..` 를 글자로만 정리한다(파일 시스템은 안 본다)
pub fn resolve(base: &Path, p: &str) -> PathBuf {
    let p = Path::new(p);
    let joined = if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
    clean(&joined)
}

pub fn clean(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                // 루트 위로는 못 올라간다
                if !matches!(out.components().next_back(), None | Some(Component::RootDir) | Some(Component::Prefix(_))) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 현재 폴더 기준 절대 경로 (path.resolve(p))
pub fn abs(p: &str) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| home());
    resolve(&cwd, p)
}

// ── 파일 종류 ──────────────────────────────────────────

pub struct Category {
    pub key: &'static str,
    pub label: &'static str,
    pub label_en: &'static str,
    pub folder: &'static str,
    pub folder_en: &'static str,
    pub extensions: &'static [&'static str],
}

pub const CATEGORIES: &[Category] = &[
    Category { key: "image", label: "사진·이미지", label_en: "Images", folder: "사진·이미지", folder_en: "Images", extensions: &["jpg", "jpeg", "png", "gif", "heic", "webp", "tiff", "bmp", "svg"] },
    Category { key: "video", label: "동영상", label_en: "Video", folder: "미디어", folder_en: "Media", extensions: &["mp4", "mov", "avi", "mkv", "webm", "m4v"] },
    Category { key: "audio", label: "음악·녹음", label_en: "Audio", folder: "미디어", folder_en: "Media", extensions: &["mp3", "m4a", "wav", "flac", "aac", "ogg"] },
    Category { key: "doc", label: "문서", label_en: "Documents", folder: "자료", folder_en: "Files", extensions: &["pdf", "hwp", "hwpx", "doc", "docx", "txt", "md", "rtf", "pages", "html"] },
    Category { key: "sheet", label: "표·슬라이드", label_en: "Sheets & slides", folder: "자료", folder_en: "Files", extensions: &["xls", "xlsx", "csv", "numbers", "ppt", "pptx", "key"] },
    Category { key: "archive", label: "압축 파일", label_en: "Archives", folder: "자료", folder_en: "Files", extensions: &["zip", "7z", "rar", "tar", "gz"] },
    Category { key: "installer", label: "설치 파일", label_en: "Installers", folder: "", folder_en: "", extensions: &["dmg", "pkg", "exe", "msi"] },
    Category { key: "dev", label: "폰트·개발", label_en: "Fonts & dev", folder: "개발리소스", folder_en: "Dev Resources", extensions: &["ttf", "otf", "woff", "woff2", "json", "yaml", "yml", "log", "xml"] },
];

fn cat(key: &str) -> &'static Category {
    CATEGORIES.iter().find(|c| c.key == key).expect("category")
}

/// 규칙 화면의 '파일 종류' 칩 (지금 언어로)
pub fn local_categories() -> Value {
    Value::Array(
        CATEGORIES
            .iter()
            .map(|c| {
                json!({
                    "key": c.key,
                    "label": l(c.label, c.label_en),
                    "folder": l(c.folder, c.folder_en),
                    "extensions": c.extensions,
                })
            })
            .collect(),
    )
}

// ── id ───────────────────────────────────────────────

static ID_SEQ: AtomicU64 = AtomicU64::new(0);

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

pub fn new_id(prefix: &str) -> String {
    let seq = ID_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    format!("{prefix}_{}{}", base36(now_ms() as u64), base36(seq))
}

// ── 설정 모양 ─────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub extensions: Vec<String>,
    pub name_includes: Vec<String>,
    /// 받은 지 N일(받은 시각과 수정 시각 중 늦은 쪽부터 센다)
    #[serde(serialize_with = "ser_num")]
    pub older_than_days: f64,
    /// 받은 곳(내려받은 주소)에 이 낱말이 들어 있으면
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub from_includes: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(rename = "match")]
    pub matcher: Match,
    pub action: String,
    pub dest: String,
    /// 목적지 안 소분류: "" 안 함 · "auto" 용도·이름·시간 · "month" 연-월
    #[serde(default)]
    pub group: String,
}

/// 소분류 방식
pub const GROUP_MODES: [&str; 3] = ["", "auto", "month"];

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: String,
    pub path: String,
    pub auto: bool,
    pub rules: Vec<Rule>,
    /// 하위 폴더 안 파일까지 정리 (기본 꺼짐: 사용자가 만든 폴더는 건드리지 않는다)
    #[serde(default)]
    pub recursive: bool,
    /// 기준 폴더(다른 정리 대상) id. 있으면 이 폴더 규칙 다음에 기준 폴더 규칙을 같이 쓴다.
    #[serde(default)]
    pub link: String,
    /// 자동 정리에서 휴지통 규칙도 실행할지 (끄면 옮기기 규칙만 자동으로 돈다)
    #[serde(default = "yes")]
    pub auto_trash: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Skip {
    pub hidden: bool,
    pub patterns: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Ai {
    pub model: String,
    /// 이미지·동영상 썸네일을 Claude 에 보내 내용으로 묶기 (기본 꺼짐)
    #[serde(default)]
    pub vision: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(serialize_with = "ser_num")]
    pub auto_delay_sec: f64,
    pub notify: bool,
    pub open_at_login: bool,
    pub smash: bool,
    pub open_after: bool,
    pub language: String,
    /// 정리할 때 타격음
    #[serde(default = "yes")]
    pub sound: bool,
    /// 자동 정리를 켜지 않은 폴더에 파일이 쌓이면 잔소리 배너
    #[serde(default = "yes")]
    pub nag: bool,
    /// 일요일 저녁 주간 성적표 배너
    #[serde(default = "yes")]
    pub weekly: bool,
    /// 첫 실행 안내를 봤는지
    #[serde(default)]
    pub intro_seen: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub version: Value,
    pub targets: Vec<Target>,
    pub skip: Skip,
    pub ai: Ai,
    pub settings: Settings,
}

fn rule(name: String, extensions: Vec<String>, dest: String, action: &str, older: f64) -> Rule {
    Rule {
        id: new_id("r"),
        name,
        enabled: true,
        matcher: Match { extensions, name_includes: vec![], older_than_days: older, from_includes: vec![] },
        action: action.into(),
        dest: if action == "trash" { String::new() } else { dest },
        group: if action == "trash" { String::new() } else { "auto".into() },
    }
}

fn exts(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// 문서 폴더를 기준으로 정리한다. 다운로드에 들어온 파일도 문서 폴더의 같은 분류로 보낸다.
fn category_rules(dest_base: &str) -> Vec<Rule> {
    let to = |c: &Category| {
        let folder = l(c.folder, c.folder_en);
        if dest_base.is_empty() { folder } else { format!("{dest_base}/{folder}") }
    };
    let media = [cat("video").extensions, cat("audio").extensions].concat();
    let docs = [cat("doc").extensions, cat("sheet").extensions, cat("archive").extensions].concat();
    vec![
        rule(l("사진·이미지", "Images"), exts(cat("image").extensions), to(cat("image")), "move", 0.0),
        rule(l("동영상·음악·녹음", "Video & audio"), exts(&media), to(cat("video")), "move", 0.0),
        rule(l("문서·표·압축 파일", "Documents, sheets & archives"), exts(&docs), to(cat("doc")), "move", 0.0),
        rule(l("폰트·개발 리소스", "Fonts & dev resources"), exts(cat("dev").extensions), to(cat("dev")), "move", 0.0),
    ]
}

/// 다운로드는 문서 폴더를 기준 폴더로 연결한다: 다운로드 규칙(오래된 설치 파일) 다음에 문서 규칙을 같이 써서
/// 다운로드에 들어온 파일도 문서 폴더의 같은 분류 트리로 모인다.
pub fn default_config() -> Config {
    let downloads = vec![rule(l("오래된 설치 파일", "Old installers"), exts(cat("installer").extensions), String::new(), "trash", 7.0)];
    let target = |id: &str, path: &str, rules: Vec<Rule>, link: &str| Target {
        id: id.into(),
        path: path.into(),
        auto: false,
        rules,
        recursive: false,
        link: link.into(),
        auto_trash: true,
    };
    Config {
        version: json!(3),
        targets: vec![target("downloads", "~/Downloads", downloads, "documents"), target("documents", "~/Documents", category_rules(""), "")],
        skip: Skip {
            hidden: true,
            patterns: exts(&[".localized", "desktop.ini", "Thumbs.db", "*.crdownload", "*.download", "*.part", "*.tmp", ".정리기록*"]),
        },
        ai: Ai { model: "sonnet".into(), vision: false },
        settings: Settings {
            auto_delay_sec: 10.0,
            notify: true,
            open_at_login: false,
            smash: true,
            open_after: true,
            language: "system".into(),
            sound: true,
            nag: true,
            weekly: true,
            intro_seen: false,
        },
    }
}

// ── 느슨하게 읽기 (JS 의 Number(x) || 기본값, x !== false 같은 규칙을 그대로) ──

fn js_number(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
        Some(Value::Bool(true)) => 1.0,
        _ => 0.0,
    }
}

fn num_or(v: Option<&Value>, fallback: f64) -> f64 {
    let n = js_number(v);
    if n == 0.0 || n.is_nan() { fallback } else { n }
}

fn js_string(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn truthy_str(v: &Value) -> Option<String> {
    let s = js_string(Some(v));
    if s.is_empty() { None } else { Some(s) }
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(truthy_str).collect(),
        _ => vec![],
    }
}

fn bool_not_false(v: Option<&Value>) -> bool {
    !matches!(v, Some(Value::Bool(false)))
}

pub fn normalize_match(m: Option<&Value>) -> Match {
    let ext = str_list(m.and_then(|m| m.get("extensions")))
        .into_iter()
        .map(|e| e.to_lowercase().trim_start_matches('.').to_string())
        .filter(|e| !e.is_empty())
        .collect();
    Match {
        extensions: ext,
        name_includes: str_list(m.and_then(|m| m.get("nameIncludes"))),
        older_than_days: js_number(m.and_then(|m| m.get("olderThanDays"))).max(0.0),
        from_includes: str_list(m.and_then(|m| m.get("fromIncludes"))).into_iter().map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect(),
    }
}

pub fn normalize_rule(r: &Value) -> Rule {
    let trash = r.get("action").and_then(Value::as_str) == Some("trash");
    Rule {
        id: r.get("id").and_then(truthy_str).unwrap_or_else(|| new_id("r")),
        name: r.get("name").and_then(truthy_str).unwrap_or_else(|| l("이름 없는 규칙", "Untitled rule")),
        enabled: bool_not_false(r.get("enabled")),
        matcher: normalize_match(r.get("match")),
        action: if trash { "trash".into() } else { "move".into() },
        dest: if trash { String::new() } else { js_string(r.get("dest")) },
        group: match r.get("group").and_then(Value::as_str) {
            Some(g) if !trash && GROUP_MODES.contains(&g) => g.into(),
            _ => String::new(),
        },
    }
}

pub fn normalize(cfg: &Value) -> Config {
    let base = default_config();
    let skip = cfg.get("skip");
    let ai = cfg.get("ai");
    let st = cfg.get("settings");
    let get_bool = |o: Option<&Value>, k: &str, d: bool| o.and_then(|o| o.get(k)).and_then(Value::as_bool).unwrap_or(d);

    let language = st.and_then(|s| s.get("language")).and_then(Value::as_str).unwrap_or("system");
    let settings = Settings {
        auto_delay_sec: num_or(st.and_then(|s| s.get("autoDelaySec")), base.settings.auto_delay_sec).clamp(3.0, 3600.0),
        notify: get_bool(st, "notify", base.settings.notify),
        open_at_login: get_bool(st, "openAtLogin", base.settings.open_at_login),
        smash: bool_not_false(st.and_then(|s| s.get("smash"))),
        open_after: bool_not_false(st.and_then(|s| s.get("openAfter"))),
        language: if ["system", "ko", "en"].contains(&language) { language.into() } else { "system".into() },
        sound: bool_not_false(st.and_then(|s| s.get("sound"))),
        nag: bool_not_false(st.and_then(|s| s.get("nag"))),
        weekly: bool_not_false(st.and_then(|s| s.get("weekly"))),
        intro_seen: get_bool(st, "introSeen", false),
    };

    let targets = match cfg.get("targets") {
        Some(Value::Array(list)) => list
            .iter()
            .map(|t| Target {
                id: t.get("id").and_then(truthy_str).unwrap_or_else(|| new_id("t")),
                path: js_string(t.get("path")),
                auto: t.get("auto") == Some(&Value::Bool(true)),
                rules: match t.get("rules") {
                    Some(Value::Array(rs)) => rs.iter().map(normalize_rule).collect(),
                    _ => vec![],
                },
                recursive: t.get("recursive") == Some(&Value::Bool(true)),
                link: js_string(t.get("link")),
                auto_trash: bool_not_false(t.get("autoTrash")),
            })
            .collect::<Vec<Target>>(),
        _ => base.targets.clone(),
    };
    // 기준 폴더는 한 단계만: 없는 폴더·자기 자신·다른 연결 폴더를 가리키면 끊는다
    let mut targets = targets;
    let bases: Vec<(String, bool)> = targets.iter().map(|t| (t.id.clone(), t.link.is_empty())).collect();
    for t in targets.iter_mut() {
        if !t.link.is_empty() && !bases.iter().any(|(id, is_base)| *id == t.link && *is_base && *id != t.id) {
            t.link.clear();
        }
    }

    Config {
        version: cfg.get("version").cloned().unwrap_or(base.version),
        targets,
        skip: Skip {
            hidden: get_bool(skip, "hidden", base.skip.hidden),
            patterns: match skip.and_then(|s| s.get("patterns")) {
                Some(Value::Array(_)) => str_list(skip.and_then(|s| s.get("patterns"))),
                _ => base.skip.patterns,
            },
        },
        ai: Ai {
            model: ai.and_then(|a| a.get("model")).and_then(truthy_str).unwrap_or(base.ai.model),
            vision: get_bool(ai, "vision", false),
        },
        settings,
    }
}

// ── 읽고 쓰기 ─────────────────────────────────────────

pub fn write_json_atomic(file: &Path, data: &impl Serialize) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // 임시 이름은 프로세스·호출마다 다르게 한다(같은 프로세스의 두 스레드가 같은 임시 파일을 쓰지 않게)
    let tmp = file.with_file_name(format!(
        "{}.{}.{}.tmp",
        file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        std::process::id(),
        new_id("w")
    ));
    let text = serde_json::to_string_pretty(data).map_err(std::io::Error::other)?;
    let written = (|| {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, file)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// 설정 파일이 바뀌었는지 가리는 값(수정 시각 + 크기). 화면이 저장 직전에 디스크와 비교한다.
pub fn config_rev() -> String {
    match std::fs::metadata(config_path()) {
        Ok(m) => {
            let ms = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
            format!("{ms}-{}", m.len())
        }
        Err(_) => String::new(),
    }
}

pub fn load_config() -> Result<Config, String> {
    let file = config_path();
    if !file.exists() {
        let cfg = default_config();
        write_json_atomic(&file, &cfg).map_err(|e| e.to_string())?;
        return Ok(cfg);
    }
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let raw: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok(normalize(&raw))
}

pub fn save_config_value(cfg: &Value) -> Result<Config, String> {
    let clean = normalize(cfg);
    write_json_atomic(&config_path(), &clean).map_err(|e| e.to_string())?;
    Ok(clean)
}

pub fn save_config(cfg: &Config) -> Result<Config, String> {
    save_config_value(&serde_json::to_value(cfg).map_err(|e| e.to_string())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::set_lang;
    use crate::test_util::LOCK;

    #[test]
    fn english_defaults() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_lang("en");
        let cfg = default_config();
        let dests: Vec<_> = cfg.targets[1].rules.iter().map(|r| r.dest.clone()).filter(|d| !d.is_empty()).collect();
        assert!(dests.contains(&"Images".to_string()));
        assert!(dests.contains(&"Files".to_string()));
        assert_eq!(cfg.targets[0].rules[0].name, "Old installers");
        assert_eq!(cfg.targets[0].link, "documents", "다운로드는 문서 폴더 규칙을 같이 쓴다");
        assert!(cfg.targets[1].rules.iter().all(|r| r.group == "auto"));
        assert_eq!(cfg.settings.language, "system");
        assert_eq!(l("한국어", "English"), "English");
        set_lang("ko");
        assert!(default_config().targets[1].rules.iter().any(|r| r.dest == "사진·이미지"));
    }

    #[test]
    fn normalize_is_lenient() {
        let cfg = normalize(&json!({
            "settings": { "autoDelaySec": "1", "smash": null, "language": "fr" },
            "targets": [{ "path": "~/x", "rules": [{ "match": { "extensions": [".PDF", ""] }, "action": "trash", "dest": "y" }] }]
        }));
        assert_eq!(cfg.settings.auto_delay_sec, 3.0);
        assert!(cfg.settings.smash);
        assert_eq!(cfg.settings.language, "system");
        let r = &cfg.targets[0].rules[0];
        assert_eq!(r.matcher.extensions, vec!["pdf"]);
        assert_eq!(r.dest, "");
        assert!(r.id.starts_with("r_"));
        assert!(cfg.targets[0].auto_trash && cfg.settings.sound && cfg.settings.nag && !cfg.settings.intro_seen, "새 설정의 기본값");
        let again = normalize(&json!({ "targets": [{ "path": "~/x", "autoTrash": false, "rules": [{ "match": { "fromIncludes": [" github.com ", ""] }, "action": "move", "dest": "y" }] }], "settings": { "sound": false } }));
        assert!(!again.targets[0].auto_trash && !again.settings.sound);
        assert_eq!(again.targets[0].rules[0].matcher.from_includes, vec!["github.com"]);
        let round = normalize(&serde_json::to_value(&again).unwrap());
        assert!(!round.targets[0].auto_trash, "저장한 모양 그대로 다시 읽힌다");
        assert_eq!(r.group, "", "휴지통 규칙은 소분류 없음");
        let text = serde_json::to_string(&cfg.settings).unwrap();
        assert!(text.contains("\"autoDelaySec\":3"), "{text}");
    }

    #[test]
    fn links_are_one_level() {
        let cfg = normalize(&json!({ "targets": [
            { "id": "a", "path": "~/a", "link": "b" },
            { "id": "b", "path": "~/b", "link": "c" },
            { "id": "c", "path": "~/c" },
            { "id": "d", "path": "~/d", "link": "d" },
            { "id": "e", "path": "~/e", "link": "없음" }
        ]}));
        let links: Vec<_> = cfg.targets.iter().map(|t| t.link.as_str()).collect();
        assert_eq!(links, vec!["", "c", "", "", ""], "연결된 폴더를 다시 기준으로 쓰거나 자기·없는 폴더를 가리키면 끊는다");
    }

    #[test]
    fn paths() {
        assert_eq!(clean(Path::new("/a/b/../c/./d")), PathBuf::from("/a/c/d"));
        assert_eq!(resolve(Path::new("/a/b"), "../x"), PathBuf::from("/a/x"));
        let h = home();
        assert_eq!(contract_home(&h.join("Docs").join("a").to_string_lossy()), "~/Docs/a");
        assert_eq!(expand_home("~/Docs"), h.join("Docs").to_string_lossy());
    }
}
