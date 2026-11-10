//! 폴더 스캔 → 정리 계획 → 실행 → 기록·되돌리기.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::config::{abs, clean, contract_home, expand_home, new_id, now_ms, resolve, ser_num, Config, Rule, Target};
use crate::fsops;
pub use crate::history::{iso_now, load_history, undo_last, undo_run, Failed, Op, Recorder, Run};
use crate::lang::l;
use crate::trash::move_many_to_trash;

const DAY_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;
/// 다른 디스크로 옮길 때 쓰는 임시 파일. 설정의 건너뛰기 목록과 상관없이 항상 건너뛴다.
const TEMP_PATTERN: &str = ".smash-*.tmp";
/// 같은 이름이 끼어들었을 때 다시 계획하는 횟수
const MOVE_RETRIES: usize = 5;
/// 폴더 목록을 만들 때 들어가지 않는 폴더(내부 구조라 정리 대상이 아님)
const OPAQUE_DIRS: [&str; 3] = ["node_modules", ".git", "Unreal Projects"];
/// 하위 폴더까지 정리할 때 들어가지 않는 폴더: 앱·라이브러리 같은 꾸러미 폴더(확장자로 판단)
const PACKAGE_EXTS: &[&str] = &[
    "app", "bundle", "framework", "plugin", "kext", "photoslibrary", "musiclibrary", "tvlibrary", "imovielibrary", "fcpbundle", "logicx", "band",
    "lrdata", "lrlibrary", "aplibrary", "xcodeproj", "xcworkspace", "playground", "rtfd", "pages", "numbers", "key", "sparsebundle", "pkg", "mpkg",
];
/// 이 파일이 들어 있으면 개발 프로젝트로 보고 안에 들어가지 않는다(파일을 빼면 프로젝트가 깨진다)
const PROJECT_MARKERS: &[&str] = &[
    ".git", "package.json", "Cargo.toml", "go.mod", "pyproject.toml", "requirements.txt", "pom.xml", "build.gradle", "build.gradle.kts",
    "settings.gradle", "Gemfile", "composer.json", "CMakeLists.txt", "Makefile", ".project", "Podfile", "pubspec.yaml",
];
/// 이 확장자의 파일이 들어 있어도 개발 프로젝트로 본다 (이름이 제각각이라 확장자로 본다: Foo.sln)
const PROJECT_MARKER_EXTS: &[&str] = &["sln", "xcodeproj", "csproj"];
const MAX_DEPTH: usize = 8;
const MAX_SCAN_FILES: usize = 20_000;

// ── 모양 ──────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Item {
    pub id: String,
    pub src: String,
    pub name: String,
    #[serde(serialize_with = "ser_num")]
    pub size: f64,
    pub action: String,
    pub dest: String,
    pub note: Option<String>,
    pub duplicate_of: Option<String>,
    pub rule_id: Option<String>,
    pub reason: String,
    /// 규칙의 대분류 폴더(소분류를 붙이기 전). 이미지 내용으로 다시 묶을 때 쓴다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_base: Option<String>,
    /// 이 규칙 뒤에 있어서 밀린, 같이 맞는 규칙 이름들 (왜 이 규칙이 잡았는지 설명용)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also_matched: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    #[serde(serialize_with = "ser_num")]
    pub size: f64,
    #[serde(serialize_with = "ser_num")]
    pub mtime_ms: f64,
    /// 받은 시각(폴더에 들어온 시각). 모르면 None 이고 "받은 지 N일" 조건에 맞지 않는다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received_ms: Option<f64>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub target_id: String,
    pub root: String,
    pub items: Vec<Item>,
    pub unmatched: Vec<FileInfo>,
    /// 규칙에는 맞지만 실행할 수 없는 항목(목적지가 정리 대상 밖). 계획에 넣지 않고 이유만 보여 준다.
    pub blocked: Vec<Item>,
    /// 파일이 너무 많아 일부만 봤다
    pub truncated: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ApplyResult {
    pub id: String,
    pub name: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub results: Vec<ApplyResult>,
    pub run: Option<Run>,
    /// 기록을 쓰지 못했을 때의 사유. 이때는 남은 파일을 옮기지 않는다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_error: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct Rejected {
    pub src: String,
    pub reason: String,
}

#[derive(Serialize, Debug)]
pub struct Planned {
    pub items: Vec<Item>,
    pub rejected: Vec<Rejected>,
}

/// AI·MCP 가 정한 이동 하나
pub struct MoveReq {
    pub src: String,
    pub dest_folder: Option<String>,
    pub action: String,
    pub reason: String,
}

// ── 도우미 ────────────────────────────────────────────

/// macOS 는 파일 이름의 한글을 자모로 풀어서(NFD) 돌려준다. 사용자가 입력한 단어(NFC)와 비교하려면 맞춰야 한다.
pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}

fn lower_nfc(s: &str) -> String {
    nfc(s).to_lowercase()
}

/// `*`·`?` 만 쓰는 간단한 glob, 대소문자 무시
#[cfg(test)]
fn glob_match(pattern: &str, name: &str) -> bool {
    glob_match_lower(&lower_nfc(pattern), &lower_nfc(name))
}

/// 둘 다 이미 소문자·NFC 인 glob 비교
fn glob_match_lower(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0usize, 0usize, None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub struct Skipper {
    hidden: bool,
    patterns: Vec<String>,
}

impl Skipper {
    pub fn new(cfg: &Config) -> Self {
        // 패턴은 한 번만 소문자·NFC 로 바꿔 둔다(파일마다 다시 바꾸지 않게)
        let patterns = cfg.skip.patterns.iter().map(|p| lower_nfc(p)).chain([TEMP_PATTERN.to_string()]).collect();
        Skipper { hidden: cfg.skip.hidden, patterns }
    }
    pub fn skip(&self, name: &str) -> bool {
        if self.hidden && name.starts_with('.') {
            return true;
        }
        let lower = lower_nfc(name);
        self.patterns.iter().any(|p| glob_match_lower(p, &lower))
    }
}

pub fn ext_of(name: &str) -> String {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn is_inside(child: &Path, parent: &Path) -> bool {
    child != parent && child.starts_with(parent)
}

pub fn target_root(t: &Target) -> PathBuf {
    abs(&expand_home(&t.path))
}

pub fn target_roots(cfg: &Config) -> Vec<PathBuf> {
    cfg.targets.iter().filter(|t| !t.path.is_empty()).map(target_root).collect()
}

pub fn inside_any_root(cfg: &Config, p: &Path) -> bool {
    let a = clean(p);
    target_roots(cfg).iter().any(|root| a == *root || is_inside(&a, root))
}

/// 실제 위치(심볼릭 링크를 따라간 경로)로도 정리 대상 안인지. 아직 없는 폴더는 있는 데까지만 따라간다.
/// 정리 대상 안의 폴더 바로가기가 바깥을 가리키면 글자만 본 검사를 통과하므로, Claude 가 정한 이동에 쓴다.
pub fn really_inside_any_root(cfg: &Config, p: &Path) -> bool {
    let mut probe = clean(p);
    let real = loop {
        match fs::canonicalize(&probe) {
            Ok(r) => break r,
            Err(_) => match probe.parent() {
                Some(up) if up != probe => probe = up.to_path_buf(),
                _ => return false,
            },
        }
    };
    target_roots(cfg).iter().filter_map(|r| fs::canonicalize(r).ok()).any(|root| real == root || real.starts_with(&root))
}

pub fn find_target<'a>(cfg: &'a Config, reference: Option<&str>) -> Option<&'a Target> {
    let Some(r) = reference.filter(|r| !r.is_empty()) else { return cfg.targets.first() };
    let wanted = abs(&expand_home(r));
    cfg.targets.iter().find(|t| t.id == r || target_root(t) == wanted)
}

fn find_target_mut<'a>(cfg: &'a mut Config, reference: &str) -> Option<&'a mut Target> {
    let wanted = abs(&expand_home(reference));
    cfg.targets.iter_mut().find(|t| t.id == reference || target_root(t) == wanted)
}

fn not_in_settings(r: &str) -> String {
    format!("{}{r}", l("설정에 없는 폴더입니다: ", "Not a folder in settings: "))
}

fn rule_has_condition(rule: &Rule) -> bool {
    let m = &rule.matcher;
    m.extensions.len() + m.name_includes.len() + m.from_includes.len() > 0 || m.older_than_days > 0.0
}

/// 파일 이름·시각만 보는 조건. "받은 지 N일"은 받은 시각과 수정 시각 중 늦은 쪽부터 센다.
/// 받은 시각을 모르면 오래됐다고 보지 않는다(방금 받은 옛 날짜 파일을 치우지 않게).
pub fn match_rule(rule: &Rule, name: &str, mtime_ms: f64, received_ms: Option<f64>, now: f64) -> bool {
    if !rule.enabled || !rule_has_condition(rule) {
        return false;
    }
    let m = &rule.matcher;
    if !m.extensions.is_empty() && !m.extensions.contains(&ext_of(name)) {
        return false;
    }
    if !m.name_includes.is_empty() {
        let lower = lower_nfc(name);
        if !m.name_includes.iter().any(|s| lower.contains(&lower_nfc(s))) {
            return false;
        }
    }
    if m.older_than_days > 0.0 {
        let Some(received) = received_ms else { return false };
        if now - received.max(mtime_ms) < m.older_than_days * DAY_MS {
            return false;
        }
    }
    true
}

/// 받은 곳(내려받은 주소) 조건까지 본다. 주소 기록은 그 조건이 있는 규칙에서만 읽는다.
fn match_file(rule: &Rule, file: &FileInfo, now: f64) -> bool {
    if !match_rule(rule, &file.name, file.mtime_ms, file.received_ms, now) {
        return false;
    }
    let words = &rule.matcher.from_includes;
    words.is_empty() || fsops::where_from(Path::new(&file.path)).is_some_and(|raw| words.iter().any(|w| fsops::from_contains(&raw, w)))
}

/// 이 규칙의 목적지가 정리 대상 폴더 안인지 (밖이면 실행 단계에서 거부된다)
pub fn rule_dest_inside(cfg: &Config, target: &Target, rule: &Rule) -> bool {
    rule.action == "trash" || inside_any_root(cfg, &resolve_dest_dir(&target_root(target), &rule.dest))
}

fn resolve_dest_dir(base: &Path, dest: &str) -> PathBuf {
    resolve(base, &expand_home(dest.trim()))
}

fn hash_file(file: &Path) -> std::io::Result<Vec<u8>> {
    let mut f = fs::File::open(file)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_vec())
}

fn same_content(a: &Path, b: &Path) -> bool {
    let (Ok(sa), Ok(sb)) = (fs::metadata(a), fs::metadata(b)) else { return false };
    sa.len() == sb.len() && matches!((hash_file(a), hash_file(b)), (Ok(x), Ok(y)) if x == y)
}

fn split_name(name: &str) -> (String, String) {
    let p = Path::new(name);
    match p.extension() {
        Some(ext) if !name.starts_with('.') || name[1..].contains('.') => {
            let ext = format!(".{}", ext.to_string_lossy());
            (name[..name.len() - ext.len()].to_string(), ext)
        }
        _ => (name.to_string(), String::new()),
    }
}

fn unique_name(dir: &Path, name: &str, reserved: &HashSet<PathBuf>) -> PathBuf {
    let (stem, ext) = split_name(name);
    let mut i = 2;
    loop {
        let candidate = dir.join(format!("{stem} ({i}){ext}"));
        if fs::symlink_metadata(&candidate).is_err() && !reserved.contains(&candidate) {
            return candidate;
        }
        i += 1;
    }
}

struct Plan {
    action: &'static str,
    dest: String,
    note: Option<&'static str>,
    duplicate_of: Option<String>,
}

/// 같은 이름이 이미 있으면: 내용까지 같으면 중복으로 휴지통, 다르면 " (2)"를 붙인다.
fn plan_move(src: &Path, dest_dir: &Path, reserved: &mut HashSet<PathBuf>) -> Option<Plan> {
    let name = src.file_name()?.to_string_lossy().into_owned();
    let dest_dir = clean(dest_dir);
    let dest = dest_dir.join(&name);
    if src.parent().map(clean) == Some(dest_dir.clone()) {
        return None;
    }
    let s = |p: &Path| p.to_string_lossy().into_owned();
    if reserved.contains(&dest) {
        let renamed = unique_name(&dest_dir, &name, reserved);
        reserved.insert(renamed.clone());
        return Some(Plan { action: "move", dest: s(&renamed), note: Some("renamed"), duplicate_of: None });
    }
    if fs::symlink_metadata(&dest).is_ok() {
        if dest.is_file() && same_content(src, &dest) {
            return Some(Plan { action: "trash", dest: String::new(), note: Some("duplicate"), duplicate_of: Some(s(&dest)) });
        }
        let renamed = unique_name(&dest_dir, &name, reserved);
        reserved.insert(renamed.clone());
        return Some(Plan { action: "move", dest: s(&renamed), note: Some("renamed"), duplicate_of: None });
    }
    reserved.insert(dest.clone());
    Some(Plan { action: "move", dest: s(&dest), note: None, duplicate_of: None })
}

fn mtime_ms(meta: &fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

pub fn list_files(dir: &Path, skip: &Skipper) -> Result<Vec<FileInfo>, String> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !ft.is_file() || skip.skip(&name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let path = entry.path();
        let received_ms = fsops::received_ms(&path, &meta);
        out.push(FileInfo { name, path: path.to_string_lossy().into_owned(), size: meta.len() as f64, mtime_ms: mtime_ms(&meta), received_ms });
    }
    out.sort_by_cached_key(|f| lower_nfc(&f.name));
    Ok(out)
}

fn make_item(file: &FileInfo, p: Plan, rule_id: Option<String>, reason: String) -> Item {
    Item {
        id: new_id("i"),
        src: file.path.clone(),
        name: file.name.clone(),
        size: file.size,
        action: p.action.into(),
        dest: p.dest,
        note: p.note.map(String::from),
        duplicate_of: p.duplicate_of,
        rule_id,
        reason,
        group_base: None,
        also_matched: vec![],
    }
}

// ── 스캔 ──────────────────────────────────────────────

/// 이 정리 대상에 적용할 규칙과, 규칙 목적지(상대 경로)의 기준이 되는 폴더.
/// 자기 규칙이 먼저, 기준 폴더에 연결돼 있으면 그다음 기준 폴더 규칙.
pub fn effective_rules<'a>(cfg: &'a Config, target: &'a Target) -> Vec<(&'a Rule, PathBuf)> {
    let root = target_root(target);
    let mut out: Vec<(&Rule, PathBuf)> = target.rules.iter().map(|r| (r, root.clone())).collect();
    if !target.link.is_empty() {
        if let Some(base) = cfg.targets.iter().find(|t| t.id == target.link && t.id != target.id) {
            let base_root = target_root(base);
            out.extend(base.rules.iter().map(|r| (r, base_root.clone())));
        }
    }
    out
}

fn is_opaque_dir(dir: &Path, name: &str) -> bool {
    let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    OPAQUE_DIRS.contains(&name)
        || name.ends_with("_files")
        || PACKAGE_EXTS.contains(&ext.as_str())
        || PROJECT_MARKERS.iter().any(|m| dir.join(m).exists())
        || fs::read_dir(dir).is_ok_and(|entries| entries.flatten().any(|e| PROJECT_MARKER_EXTS.contains(&ext_of(&e.file_name().to_string_lossy()).as_str())))
}

/// 정리 대상의 파일들. 하위 폴더까지 정리가 꺼져 있으면 바로 아래 파일만.
/// 켜져 있어도 숨김·건너뛰기 폴더, 앱·라이브러리 꾸러미, 개발 프로젝트, 다른 정리 대상 폴더 안에는 들어가지 않는다.
pub fn scan_files(cfg: &Config, target: &Target) -> Result<Vec<FileInfo>, String> {
    scan_files_capped(cfg, target).map(|(files, _)| files)
}

/// 파일 목록과, 상한(20,000개·깊이 8)에 걸려 다 보지 못했는지
fn scan_files_capped(cfg: &Config, target: &Target) -> Result<(Vec<FileInfo>, bool), String> {
    let root = target_root(target);
    let skip = Skipper::new(cfg);
    let mut out = list_files(&root, &skip)?;
    if !target.recursive {
        return Ok((out, false));
    }
    let mut truncated = false;
    let others: Vec<PathBuf> = target_roots(cfg).into_iter().filter(|r| *r != root).collect();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.clone(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= MAX_SCAN_FILES {
            truncated = true;
            continue;
        }
        if depth >= MAX_DEPTH {
            // 더 깊은 폴더가 있을 때만 "다 못 봤다"
            truncated |= fs::read_dir(&dir).is_ok_and(|es| es.flatten().any(|e| e.file_type().is_ok_and(|t| t.is_dir())));
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        let mut subdirs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)) // 바로가기(심볼릭 링크)는 따라가지 않는다
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let path = e.path();
                (!skip.skip(&name) && !is_opaque_dir(&path, &name) && !others.contains(&clean(&path))).then_some(path)
            })
            .collect();
        subdirs.sort();
        for sub in subdirs.into_iter().rev() {
            let room = MAX_SCAN_FILES.saturating_sub(out.len());
            if room == 0 {
                truncated = true;
                break;
            }
            if let Ok(files) = list_files(&sub, &skip) {
                truncated |= files.len() > room;
                out.extend(files.into_iter().take(room));
            }
            stack.push((sub, depth + 1));
        }
    }
    out.truncate(MAX_SCAN_FILES);
    Ok((out, truncated))
}

/// 규칙 편집 중 미리보기는 글자를 칠 때마다 불린다. 하위 폴더까지 훑는 목록은 잠깐(5초) 재사용한다.
fn scan_files_cached(cfg: &Config, target: &Target) -> Result<Vec<FileInfo>, String> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    type Cache = Option<(String, Instant, Vec<FileInfo>)>;
    static CACHE: Mutex<Cache> = Mutex::new(None);
    if !target.recursive {
        return scan_files(cfg, target);
    }
    let key = format!("{}|{}", target.id, target_root(target).display());
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, at, files)) = cache.as_ref() {
        if *k == key && at.elapsed() < Duration::from_secs(5) {
            return Ok(files.clone());
        }
    }
    let files = scan_files(cfg, target)?;
    *cache = Some((key, Instant::now(), files.clone()));
    Ok(files)
}

fn existing_subdirs(dir: &Path, skip: &Skipper) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return vec![] };
    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !skip.skip(n))
        .collect()
}

pub fn scan_target(cfg: &Config, reference: Option<&str>) -> Result<Scan, String> {
    scan_target_with(cfg, reference, true)
}

/// `subfolders`: false 면 '하위 폴더까지 정리'가 켜져 있어도 바로 아래 파일만 본다(자동 정리는 새로 들어온 파일만 다룬다).
pub fn scan_target_with(cfg: &Config, reference: Option<&str>, subfolders: bool) -> Result<Scan, String> {
    let found = find_target(cfg, reference).ok_or_else(|| not_in_settings(reference.unwrap_or("")))?;
    let mut shallow;
    let target = if subfolders || !found.recursive {
        found
    } else {
        shallow = found.clone();
        shallow.recursive = false;
        &shallow
    };
    let root = target_root(target);
    if !root.exists() {
        return Err(format!("{}{}", l("폴더가 없습니다: ", "Folder not found: "), target.path));
    }
    let skip = Skipper::new(cfg);
    let rules = effective_rules(cfg, target);
    let now = now_ms();
    let (files, truncated) = scan_files_capped(cfg, target)?;

    // 1. 파일마다 처음 맞는 규칙 (뒤에 있어 밀린 규칙 이름도 같이 모은다)
    let mut hit: Vec<Option<(&Rule, PathBuf, Vec<String>)>> = Vec::with_capacity(files.len());
    let mut unmatched = Vec::new();
    for file in &files {
        let mut matching = rules.iter().filter(|(r, _)| match_file(r, file, now));
        match matching.next() {
            Some((rule, base)) => {
                let dir = if rule.action == "trash" { PathBuf::new() } else { clean(&resolve_dest_dir(base, &rule.dest)) };
                hit.push(Some((rule, dir, matching.take(3).map(|(r, _)| r.name.clone()).collect())));
            }
            None => {
                hit.push(None);
                unmatched.push(file.clone());
            }
        }
    }

    // 2. 같은 규칙·같은 대분류 폴더로 가는 파일끼리 소분류
    let mut groups: HashMap<(String, PathBuf), Vec<usize>> = HashMap::new();
    for (i, h) in hit.iter().enumerate() {
        if let Some((rule, dir, _)) = h {
            if rule.action == "move" && !rule.group.is_empty() {
                groups.entry((rule.id.clone(), dir.clone())).or_default().push(i);
            }
        }
    }
    let mut sub: Vec<Option<String>> = vec![None; files.len()];
    for ((_, dir), idx) in &groups {
        let mode = hit[idx[0]].as_ref().map(|(r, _, _)| r.group.as_str()).unwrap_or("");
        let gfiles: Vec<crate::grouping::GFile> =
            idx.iter().map(|&i| crate::grouping::GFile { name: files[i].name.clone(), mtime_ms: files[i].mtime_ms }).collect();
        for (k, s) in crate::grouping::subfolders(mode, &gfiles, &existing_subdirs(dir, &skip)).into_iter().enumerate() {
            sub[idx[k]] = s.and_then(|s| crate::grouping::clean_folder(&s));
        }
    }

    // 3. 파일 순서대로 계획 (같은 이름 충돌을 순서대로 막는다)
    let mut reserved = HashSet::new();
    let mut items = Vec::new();
    let mut blocked = Vec::new();
    for (i, file) in files.iter().enumerate() {
        let Some((rule, dir, also)) = &hit[i] else { continue };
        let src = Path::new(&file.path);
        let planned = if rule.action == "trash" {
            Some(Plan { action: "trash", dest: String::new(), note: None, duplicate_of: None })
        } else {
            // 이미 대분류 폴더의 하위 폴더 어딘가에 있으면 그대로 둔다: 사용자가 나눠 둔 것을 풀지 않고,
            // 시간·이름 묶음이 다시 계산돼도 정리할 때마다 자리가 바뀌지 않게
            if src.parent().map(|p| is_inside(&clean(p), dir)).unwrap_or(false) {
                continue;
            }
            let dest_dir = match &sub[i] {
                Some(s) => dir.join(s),
                None => dir.clone(),
            };
            // 실행 단계는 정리 대상 밖으로 옮기지 않는다. 여기서 미리 걸러 이유를 보여 준다.
            if !inside_any_root(cfg, &dest_dir) {
                let dest = dest_dir.join(&file.name).to_string_lossy().into_owned();
                let mut item = make_item(file, Plan { action: "move", dest, note: Some("outside"), duplicate_of: None }, Some(rule.id.clone()), rule.name.clone());
                item.also_matched = also.clone();
                blocked.push(item);
                continue;
            }
            plan_move(src, &dest_dir, &mut reserved)
        };
        if let Some(p) = planned {
            let mut item = make_item(file, p, Some(rule.id.clone()), rule.name.clone());
            if rule.action == "move" {
                item.group_base = Some(dir.to_string_lossy().into_owned());
            }
            item.also_matched = also.clone();
            items.push(item);
        }
    }
    Ok(Scan { target_id: target.id.clone(), root: root.to_string_lossy().into_owned(), items, unmatched, blocked, truncated })
}

/// 미리보기 계획의 일부 항목을 새 소분류로 옮겨 다시 계획한다(이미지 내용으로 묶기 결과).
/// `assign`: 항목 id → 대분류 폴더 안 소분류("게임/엘든 링"). 나머지 항목은 원래 목적지 그대로.
pub fn regroup(cfg: &Config, items: Vec<Item>, assign: &HashMap<String, String>) -> Vec<Item> {
    let mut reserved = HashSet::new();
    let mut out = Vec::new();
    for mut item in items {
        if item.action != "move" {
            out.push(item);
            continue;
        }
        let src = abs(&item.src);
        let mut dir = abs(&item.dest).parent().map(Path::to_path_buf).unwrap_or_default();
        if let (Some(sub), Some(base)) = (assign.get(&item.id).and_then(|s| crate::grouping::clean_folder(s)), item.group_base.as_ref()) {
            let wanted = clean(&abs(base).join(sub));
            if inside_any_root(cfg, &wanted) {
                dir = wanted;
            }
        }
        if let Some(p) = plan_move(&src, &dir, &mut reserved) {
            item.action = p.action.into();
            item.dest = p.dest;
            item.note = p.note.map(String::from);
            item.duplicate_of = p.duplicate_of;
            out.push(item);
        }
    }
    out
}

/// 규칙 편집 중 미리보기: 이 규칙 하나에 지금 맞는 파일들.
pub fn test_rule(cfg: &Config, reference: &str, rule: &Value) -> Result<Vec<String>, String> {
    let target = find_target(cfg, Some(reference)).ok_or_else(|| not_in_settings(reference))?;
    let mut candidate = crate::config::normalize_rule(rule);
    candidate.enabled = true;
    let now = now_ms();
    Ok(scan_files_cached(cfg, target)?
        .into_iter()
        .filter(|f| match_file(&candidate, f, now))
        .map(|f| f.name)
        .collect())
}

/// 아직 없는 목적지 폴더들(미리보기에서 '새 폴더' 표시용).
pub fn missing_dirs(dest: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut dir = dest.parent().map(Path::to_path_buf);
    while let Some(d) = dir {
        if d.as_os_str().is_empty() || d.exists() {
            break;
        }
        out.push(d.clone());
        dir = d.parent().map(Path::to_path_buf).filter(|p| *p != d);
    }
    out
}

/// AI·MCP 가 만든 이동 목록을 검증해서 실행 가능한 계획으로 바꾼다.
pub fn plan_from_moves(cfg: &Config, moves: Vec<MoveReq>) -> Planned {
    let mut reserved = HashSet::new();
    let mut items = Vec::new();
    let mut rejected = Vec::new();
    for m in moves {
        let src = abs(&expand_home(&m.src));
        let mut fail = |why: String| rejected.push(Rejected { src: m.src.clone(), reason: why });
        if !inside_any_root(cfg, &src) {
            fail(l("설정한 정리 대상 폴더 밖의 파일입니다", "The file is outside the folders you set up"));
            continue;
        }
        let Ok(st) = fs::symlink_metadata(&src) else {
            fail(l("파일이 없습니다", "File not found"));
            continue;
        };
        if !st.is_file() {
            fail(l("파일만 옮길 수 있습니다", "Only files can be moved"));
            continue;
        }
        // 정리 대상 안의 폴더 바로가기를 거쳐 바깥 파일을 가리키는 경우를 막는다
        if !src.parent().is_some_and(|d| really_inside_any_root(cfg, d)) {
            fail(l("설정한 정리 대상 폴더 밖의 파일입니다", "The file is outside the folders you set up"));
            continue;
        }
        let file = FileInfo {
            name: src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            path: src.to_string_lossy().into_owned(),
            size: st.len() as f64,
            mtime_ms: mtime_ms(&st),
            received_ms: None,
        };
        if m.action == "trash" {
            items.push(make_item(&file, Plan { action: "trash", dest: String::new(), note: None, duplicate_of: None }, None, m.reason));
            continue;
        }
        let base = src.parent().map(Path::to_path_buf).unwrap_or_default();
        let dest_dir = resolve_dest_dir(&base, m.dest_folder.as_deref().unwrap_or(""));
        if !inside_any_root(cfg, &dest_dir) || !really_inside_any_root(cfg, &dest_dir) {
            fail(l("목적지가 정리 대상 폴더 밖입니다", "The destination is outside the folders you set up"));
            continue;
        }
        match plan_move(&src, &dest_dir, &mut reserved) {
            Some(p) => items.push(make_item(&file, p, None, m.reason)),
            None => fail(l("이미 그 폴더에 있습니다", "Already in that folder")),
        }
    }
    Planned { items, rejected }
}

// ── 실행 ──────────────────────────────────────────────

fn outside_file() -> String {
    l("정리 대상 폴더 밖의 파일입니다", "The file is outside the folders you set up")
}

/// 계획대로 옮기고 기록 하나를 남긴다.
pub fn apply_plan(cfg: &Config, items: Vec<Item>, source: &str) -> Applied {
    let mut rec = Recorder::begin(source);
    let results = apply_into(cfg, items, &mut rec);
    let (run, history_error) = rec.finish();
    Applied { results, run, history_error }
}

/// 정리 대상 전부를 규칙대로 정리한다. 폴더가 여러 개여도 기록은 실행 하나다(한 번에 되돌릴 수 있게).
pub fn apply_all(cfg: &Config, source: &str) -> Applied {
    let mut rec = Recorder::begin(source);
    let mut results = Vec::new();
    for t in &cfg.targets {
        match scan_target(cfg, Some(&t.id)) {
            Ok(scan) => results.extend(apply_into(cfg, scan.items, &mut rec)),
            Err(e) => eprintln!("{}: {e}", t.path),
        }
    }
    let (run, history_error) = rec.finish();
    Applied { results, run, history_error }
}

/// 이 폴더 바로 아래에서 규칙에 걸리는 파일 수 (잔소리용. 계획은 세우지 않는다)
pub fn count_matching(cfg: &Config, target: &Target) -> usize {
    let rules = effective_rules(cfg, target);
    let now = now_ms();
    list_files(&target_root(target), &Skipper::new(cfg)).map(|files| files.iter().filter(|f| rules.iter().any(|(r, _)| match_file(r, f, now))).count()).unwrap_or(0)
}

/// 이미 열어 둔 기록에 이어서 실행한다(여러 폴더를 한 번의 실행으로 묶을 때).
///
/// 파일마다 "무엇을 어디로"를 먼저 기록에 내려쓴 뒤에 옮긴다. 기록을 쓰지 못하면 그 파일부터는 옮기지 않는다.
/// 휴지통으로 보낼 파일은 모아서 마지막에 한 번에 보낸다.
pub fn apply_into(cfg: &Config, items: Vec<Item>, rec: &mut Recorder) -> Vec<ApplyResult> {
    let mut results: Vec<ApplyResult> = Vec::with_capacity(items.len());
    let mut reserved = HashSet::new();
    // (결과 번호, 기록 번호, 파일, 성공했을 때 문구)
    let mut to_trash: Vec<(usize, usize, PathBuf, String)> = Vec::new();
    let total = items.len();
    for mut item in items {
        #[cfg(test)]
        crate::test_util::fault_point();
        if let Some(tell) = rec.progress.as_mut() {
            tell(results.len(), total);
        }
        let src = abs(&item.src);
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let from = src.to_string_lossy().into_owned();
        let slot = results.len();
        let outcome: Result<String, String> = (|| {
            if !inside_any_root(cfg, &src) {
                return Err(outside_file());
            }
            let st = fs::symlink_metadata(&src).map_err(|e| e.to_string())?;
            if !st.is_file() {
                return Err(l("파일이 아닙니다", "Not a file"));
            }
            let size = st.len();
            // 중복이라 버리기로 한 파일은 원본이 아직 그대로 있을 때만 버린다. 아니면 원본 자리로 옮긴다.
            if item.action == "trash" {
                if let Some(orig) = item.duplicate_of.clone().filter(|s| !s.is_empty()) {
                    let original = abs(&orig);
                    let still = original.is_file() && same_content(&src, &original);
                    if !still {
                        item.action = "move".into();
                        item.dest = original.to_string_lossy().into_owned();
                    }
                }
            }
            let mut trash = |rec: &mut Recorder, message: String| -> Result<String, String> {
                let i = rec.intent(Op { kind: "trash".into(), from: from.clone(), size, ..Default::default() })?;
                to_trash.push((slot, i, src.clone(), message));
                Ok(String::new())
            };
            if item.action == "trash" {
                return trash(rec, l("휴지통으로 보냄", "Moved to Trash"));
            }
            let wanted = abs(&item.dest);
            if !inside_any_root(cfg, &wanted) {
                return Err(l("목적지가 정리 대상 폴더 밖입니다", "The destination is outside the folders you set up"));
            }
            // 계획 이후에 상황이 바뀌었을 수 있어 실행 직전에 다시 확인한다.
            let dir = wanted.parent().map(Path::to_path_buf).unwrap_or_default();
            for _ in 0..MOVE_RETRIES {
                let planned = plan_move(&src, &dir, &mut reserved).ok_or_else(|| l("이미 그 폴더에 있습니다", "Already in that folder"))?;
                if planned.action == "trash" {
                    return trash(rec, l("같은 파일이 이미 있어 휴지통으로 보냄", "Same file already there, moved to Trash"));
                }
                let dest = PathBuf::from(&planned.dest);
                let new_dirs = missing_dirs(&dest);
                let i = rec.intent(Op { kind: "move".into(), from: from.clone(), to: Some(planned.dest.clone()), size, ..Default::default() })?;
                #[cfg(test)]
                crate::test_util::before_move(&dest);
                match fsops::move_file(&src, &dest) {
                    Ok(()) => {
                        for d in new_dirs.iter().rev() {
                            rec.mkdir(d);
                        }
                        return Ok(format!("→ {}", contract_home(&planned.dest)));
                    }
                    // 확인과 이동 사이에 같은 이름이 생겼다: 덮어쓰지 않고 다른 이름으로 다시 계획한다
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => rec.void(i),
                    Err(e) => {
                        rec.void(i);
                        return Err(e.to_string());
                    }
                }
            }
            Err(l("같은 이름 파일이 계속 생겨 옮기지 못했습니다", "A file with the same name kept appearing"))
        })();
        let (ok, message) = match outcome {
            Ok(m) => (true, m),
            Err(m) => {
                rec.fail(&from, &m);
                (false, m)
            }
        };
        results.push(ApplyResult { id: item.id.clone(), name, ok, message });
    }
    if let Some(tell) = rec.progress.as_mut() {
        tell(total, total);
    }
    // 휴지통: 한 번에 보내고, 휴지통 안 위치를 기록에 덧붙인다(되돌리기에 쓴다)
    let paths: Vec<PathBuf> = to_trash.iter().map(|(_, _, p, _)| p.clone()).collect();
    for ((slot, i, path, message), res) in to_trash.into_iter().zip(move_many_to_trash(&paths)) {
        match res {
            Ok(place) => {
                rec.done(i, place.map(|p| p.to_string_lossy().into_owned()));
                results[slot].message = message;
            }
            Err(e) => {
                rec.void(i);
                rec.fail(&path.to_string_lossy(), &e);
                results[slot].ok = false;
                results[slot].message = e;
            }
        }
    }
    results
}

/// AI 가 고를 수 있는 폴더 목록(정리 대상 폴더와 그 하위 폴더).
pub fn folder_tree(cfg: &Config, depth: usize) -> Vec<String> {
    let skip = Skipper::new(cfg);
    let mut out = Vec::new();
    fn walk(dir: &Path, level: usize, depth: usize, skip: &Skipper, out: &mut Vec<String>) {
        out.push(dir.to_string_lossy().into_owned());
        let base = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if level >= depth || OPAQUE_DIRS.contains(&base.as_str()) || dir.join(".git").exists() {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else { return };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !skip.skip(n) && !n.ends_with(".app") && !n.ends_with("_files"))
            .collect();
        names.sort();
        for n in names {
            walk(&dir.join(n), level + 1, depth, skip, out);
        }
    }
    for root in target_roots(cfg) {
        if root.exists() {
            walk(&root, 0, depth, &skip, &mut out);
        }
    }
    out
}

// ── MCP 가 규칙을 더할 때 ─────────────────────────────

pub fn add_rule(cfg: &mut Config, target: &str, rule: Rule, last: bool) -> Result<String, String> {
    let t = find_target_mut(cfg, target).ok_or_else(|| not_in_settings(target))?;
    let path = t.path.clone();
    if last {
        t.rules.push(rule);
    } else {
        t.rules.insert(0, rule);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{load_config, save_config_value};
    use crate::lang::set_lang;
    use serde_json::json;
    use crate::test_util::{sandbox, set_before_move, write, write_arrived, ABORT_AFTER, LOCK};

    fn setup() -> (Config, PathBuf, PathBuf) {
        let sb = sandbox();
        let inbox = sb.join("inbox");
        let archive = sb.join("archive");
        let _ = fs::remove_dir_all(&inbox);
        let _ = fs::remove_dir_all(&archive);
        let _ = fs::remove_dir_all(sb.join("config"));
        write(&inbox.join("photo.JPG"), "img", 0);
        write(&inbox.join("report.pdf"), "new report", 0);
        write(&inbox.join("same.pdf"), "identical", 0);
        write(&inbox.join("연봉 계약서.pdf"), "contract", 0);
        write(&inbox.join("old-installer.dmg"), "dmg", 30);
        write(&inbox.join("new-installer.dmg"), "dmg2", 0);
        write(&inbox.join("mystery.bin"), "???", 0);
        write(&inbox.join(".hidden"), "x", 0);
        write(&inbox.join("video.crdownload"), "partial", 0);
        write(&inbox.join("문서").join("report.pdf"), "old report", 0);
        write(&inbox.join("문서").join("same.pdf"), "identical", 0);
        fs::create_dir_all(&archive).unwrap();
        let cfg = save_config_value(&json!({
            "targets": [
                { "id": "inbox", "path": inbox, "rules": [
                    { "name": "계약서", "match": { "nameIncludes": ["계약서"] }, "action": "move", "dest": archive },
                    { "name": "오래된 설치 파일", "match": { "extensions": ["dmg"], "olderThanDays": 7 }, "action": "trash" },
                    { "name": "이미지", "match": { "extensions": ["jpg"] }, "action": "move", "dest": "이미지" },
                    { "name": "문서", "match": { "extensions": ["pdf"] }, "action": "move", "dest": "문서" },
                    { "name": "조건 없는 규칙", "match": {}, "action": "move", "dest": "어디든" }
                ]},
                { "id": "archive", "path": archive, "rules": [] }
            ]
        }))
        .unwrap();
        (cfg, inbox, archive)
    }

    fn guard() -> std::sync::MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_lang("ko");
        g
    }

    fn rule_with(m: Value) -> Rule {
        crate::config::normalize_rule(&json!({ "name": "t", "match": m, "action": "move", "dest": "x" }))
    }

    #[test]
    fn no_condition_matches_nothing() {
        assert!(!match_rule(&rule_with(json!({})), "a.txt", 0.0, Some(0.0), now_ms()));
    }

    #[test]
    fn nfd_korean_name_matches() {
        let name: String = "스크린샷 2026-11-01 오후 9.20.11.png".nfd().collect();
        assert!(match_rule(&rule_with(json!({ "extensions": ["png"], "nameIncludes": ["스크린샷"] })), &name, 0.0, None, now_ms()));
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.crdownload", "video.CRDOWNLOAD"));
        assert!(glob_match(".정리기록*", ".정리기록 1"));
        assert!(!glob_match("*.part", "a.parts"));
        assert!(glob_match("Thumbs.db", "thumbs.db"));
    }

    #[test]
    fn scan_order_duplicates_conflicts_skips() {
        let _g = guard();
        let (cfg, inbox, archive) = setup();
        let scan = scan_target(&cfg, Some("inbox")).unwrap();
        let by = |n: &str| scan.items.iter().find(|i| i.name == n).cloned();
        let s = |p: PathBuf| p.to_string_lossy().into_owned();
        assert_eq!(by("photo.JPG").unwrap().dest, s(inbox.join("이미지").join("photo.JPG")), "확장자는 대소문자 무시");
        assert_eq!(by("연봉 계약서.pdf").unwrap().dest, s(archive.join("연봉 계약서.pdf")), "먼저 나온 규칙이 이긴다");
        let report = by("report.pdf").unwrap();
        assert_eq!(report.note.as_deref(), Some("renamed"));
        assert_eq!(report.dest, s(inbox.join("문서").join("report (2).pdf")));
        let same = by("same.pdf").unwrap();
        assert_eq!(same.action, "trash");
        assert_eq!(same.note.as_deref(), Some("duplicate"));
        assert_eq!(by("old-installer.dmg").unwrap().action, "trash");
        assert!(by("new-installer.dmg").is_none(), "7일 안 된 설치 파일은 그대로");
        let mut un: Vec<_> = scan.unmatched.iter().map(|f| f.name.clone()).collect();
        un.sort();
        assert_eq!(un, vec!["mystery.bin", "new-installer.dmg"]);
        assert!(!scan.items.iter().map(|i| &i.name).chain(scan.unmatched.iter().map(|f| &f.name)).any(|n| n.starts_with('.') || n.ends_with(".crdownload")));
    }

    #[test]
    fn apply_then_undo() {
        let _g = guard();
        let (cfg, inbox, _) = setup();
        let moves: Vec<Item> = scan_target(&cfg, Some("inbox")).unwrap().items.into_iter().filter(|i| i.action == "move").collect();
        let n = moves.len();
        let applied = apply_plan(&cfg, moves, "test");
        assert!(applied.results.iter().all(|r| r.ok), "{:?}", applied.results);
        assert!(inbox.join("이미지").join("photo.JPG").exists());
        assert!(inbox.join("문서").join("report (2).pdf").exists());
        assert_eq!(fs::read_to_string(inbox.join("문서").join("report.pdf")).unwrap(), "old report", "기존 파일은 덮어쓰지 않는다");
        assert_eq!(load_history().last().unwrap().id, applied.run.unwrap().id);
        let undo = undo_last();
        assert_eq!(undo["restored"], json!(n));
        assert!(inbox.join("photo.JPG").exists());
        assert!(inbox.join("연봉 계약서.pdf").exists());
        assert_eq!(undo_last()["ok"], json!(false), "두 번 되돌리지 않는다");
        assert!(!inbox.join("이미지").exists(), "이 실행이 만든 빈 폴더는 되돌릴 때 지운다");
        assert!(inbox.join("문서").exists(), "원래 있던 폴더는 그대로");
    }

    /// 폴더 하나에 pdf 규칙 하나짜리 설정
    fn simple(name: &str, files: &[&str]) -> (Config, PathBuf) {
        let sb = sandbox();
        let inbox = sb.join(name);
        let _ = fs::remove_dir_all(&inbox);
        let _ = fs::remove_dir_all(sb.join("config"));
        let _ = fs::remove_dir_all(sb.join("trash"));
        for f in files {
            write(&inbox.join(f), f, 0);
        }
        let cfg = save_config_value(&json!({ "targets": [{ "id": "inbox", "path": inbox, "rules": [
            { "name": "문서", "match": { "extensions": ["pdf"] }, "action": "move", "dest": "문서" }
        ]}]}))
        .unwrap();
        (cfg, inbox)
    }

    /// 아래 시험이 띄우는 자식 프로세스: 정리하다가 도중에 그대로 죽는다.
    #[test]
    fn crash_child() {
        let Ok(n) = std::env::var("SMASH_CRASH_AFTER") else { return };
        let _ = sandbox();
        let cfg = load_config().unwrap();
        let items = scan_target(&cfg, Some("inbox")).unwrap().items;
        ABORT_AFTER.store(n.parse().unwrap(), std::sync::atomic::Ordering::SeqCst);
        apply_plan(&cfg, items, "test");
        unreachable!("도중에 죽어야 한다");
    }

    #[test]
    fn crash_midway_keeps_what_was_moved_undoable() {
        let _g = guard();
        let (_cfg, inbox) = simple("crash", &["a.pdf", "b.pdf", "c.pdf", "d.pdf"]);
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["organizer::tests::crash_child", "--exact", "--test-threads=1"])
            .env("SMASH_TEST_SANDBOX", sandbox())
            .env("SMASH_CRASH_AFTER", "2")
            .output()
            .unwrap();
        assert!(!out.status.success(), "자식은 도중에 죽어야 한다");
        let moved = fs::read_dir(inbox.join("문서")).unwrap().count();
        assert_eq!(moved, 2, "죽기 전에 두 개를 옮겼다");
        let runs = load_history();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].state, "interrupted", "끝 줄이 없고 잠금도 없으면 중단된 정리");
        assert_eq!(runs[0].ops.len(), 2);
        let undo = undo_last();
        assert_eq!(undo["restored"], json!(2), "{undo}");
        for f in ["a.pdf", "b.pdf", "c.pdf", "d.pdf"] {
            assert!(inbox.join(f).exists(), "{f}");
        }
        assert!(!inbox.join("문서").exists());
    }

    #[test]
    fn concurrent_runs_keep_every_history() {
        let _g = guard();
        let names: Vec<String> = (0..40).map(|i| format!("f{i}.pdf")).collect();
        let (cfg, inbox) = simple("race", &names.iter().map(String::as_str).collect::<Vec<_>>());
        let items = scan_target(&cfg, Some("inbox")).unwrap().items;
        let (a, b): (Vec<(usize, Item)>, Vec<(usize, Item)>) = items.into_iter().enumerate().partition(|(i, _)| i % 2 == 0);
        let run = |list: Vec<(usize, Item)>, cfg: Config| {
            std::thread::spawn(move || {
                for (_, it) in list {
                    assert!(apply_plan(&cfg, vec![it], "test").results[0].ok);
                }
            })
        };
        let (t1, t2) = (run(a, cfg.clone()), run(b, cfg.clone()));
        t1.join().unwrap();
        t2.join().unwrap();
        let runs = load_history();
        assert_eq!(runs.len(), 40, "두 스레드가 20번씩 정리한 기록이 전부 남는다");
        assert!(runs.iter().all(|r| r.state == "done" && r.ops.len() == 1));
        // 설정 저장도 겹쳐도 깨지지 않는다(임시 파일 이름이 호출마다 다르다)
        let savers: Vec<_> = (0..8).map(|_| { let c = cfg.clone(); std::thread::spawn(move || crate::config::save_config(&c).unwrap()) }).collect();
        savers.into_iter().for_each(|t| { t.join().unwrap(); });
        assert_eq!(load_config().unwrap().targets.len(), 1);
        assert!(fs::read_dir(crate::config::config_dir()).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")));
        assert_eq!(fs::read_dir(inbox.join("문서")).unwrap().count(), 40);
    }

    #[test]
    fn failed_undo_stays_open_for_retry() {
        let _g = guard();
        let (cfg, inbox) = simple("partial", &["a.pdf", "b.pdf"]);
        apply_plan(&cfg, scan_target(&cfg, Some("inbox")).unwrap().items, "test");
        write(&inbox.join("a.pdf"), "blocker", 0);
        let first = undo_last();
        assert_eq!((first["restored"].clone(), first["left"].clone()), (json!(1), json!(1)), "{first}");
        assert_eq!(fs::read_to_string(inbox.join("a.pdf")).unwrap(), "blocker", "원래 자리의 다른 파일은 덮어쓰지 않는다");
        let run = load_history().pop().unwrap();
        assert!(!run.undone && run.tried && run.retryable() == 1, "실패한 것만 남는다");
        fs::remove_file(inbox.join("a.pdf")).unwrap();
        let second = undo_last();
        assert_eq!(second["restored"], json!(1), "{second}");
        assert_eq!(fs::read_to_string(inbox.join("a.pdf")).unwrap(), "a.pdf");
        assert!(load_history().pop().unwrap().undone);
        assert_eq!(undo_last()["ok"], json!(false));
    }

    #[test]
    fn tidy_everything_is_one_run_and_runs_can_be_picked() {
        let _g = guard();
        let sb = sandbox();
        let (one, two) = (sb.join("all-1"), sb.join("all-2"));
        for d in [&one, &two] {
            let _ = fs::remove_dir_all(d);
        }
        let _ = fs::remove_dir_all(sb.join("config"));
        let _ = fs::remove_dir_all(sb.join("trash"));
        write(&one.join("a.pdf"), "a", 0);
        write(&one.join("old.dmg"), "dmg", 30);
        write(&two.join("b.pdf"), "b", 0);
        let rules = json!([
            { "name": "문서", "match": { "extensions": ["pdf"] }, "action": "move", "dest": "문서" },
            { "name": "오래된 설치 파일", "match": { "extensions": ["dmg"], "olderThanDays": 7 }, "action": "trash" }
        ]);
        let cfg = save_config_value(&json!({ "targets": [{ "id": "one", "path": one, "rules": rules }, { "id": "two", "path": two, "rules": rules }] })).unwrap();
        let all = apply_all(&cfg, "app");
        assert_eq!(all.results.iter().filter(|r| r.ok).count(), 3, "{:?}", all.results);
        assert_eq!(load_history().len(), 1, "폴더가 둘이어도 기록은 하나");
        let first = all.run.unwrap();
        let trash_op = first.ops.iter().find(|o| o.kind == "trash").unwrap();
        assert!(trash_op.to.as_deref().is_some_and(|p| Path::new(p).exists()), "휴지통 안 위치를 기록한다");
        assert_eq!(trash_op.size, 3);
        // 그 뒤에 다른 정리가 한 번 더 돌았다 (자동 정리가 끼어든 경우)
        write(&two.join("c.pdf"), "c", 0);
        let later = apply_plan(&cfg, scan_target(&cfg, Some("two")).unwrap().items, "auto").run.unwrap();
        // 먼저 한 정리를 골라서 되돌린다: 두 폴더 모두, 휴지통으로 간 것까지 돌아온다
        let undo = undo_run(Some(&first.id));
        assert_eq!((undo["restored"].clone(), undo["restoredFromTrash"].clone(), undo["left"].clone()), (json!(3), json!(1), json!(0)), "{undo}");
        assert!(one.join("a.pdf").exists() && one.join("old.dmg").exists() && two.join("b.pdf").exists());
        assert!(two.join("문서").join("c.pdf").exists(), "나중 실행은 건드리지 않는다");
        assert!(!one.join("문서").exists(), "비게 된 새 폴더는 지운다");
        assert!(two.join("문서").exists(), "다른 파일이 남은 폴더는 그대로");
        let runs = load_history();
        assert_eq!(runs.iter().map(|r| (r.id.as_str(), r.undone)).collect::<Vec<_>>(), vec![(first.id.as_str(), true), (later.id.as_str(), false)]);
        assert_eq!(undo_last()["runId"], json!(later.id), "마지막 되돌리기는 아직 안 돌린 가장 최근 실행");
    }

    #[test]
    fn age_counts_from_arrival_not_from_file_date() {
        let _g = guard();
        let sb = sandbox();
        let inbox = sb.join("arrival");
        let _ = fs::remove_dir_all(&inbox);
        let _ = fs::remove_dir_all(sb.join("config"));
        write_arrived(&inbox.join("just-received.pkg"), "pkg", 2000, Some(0)); // 수정 시각은 5년 전, 받은 건 방금 (실제 값)
        write_arrived(&inbox.join("received-yesterday.pkg"), "pkg", 2000, Some(1));
        write_arrived(&inbox.join("edited-yesterday.pkg"), "pkg", 1, Some(40)); // 오래전에 받았지만 어제 고쳤다
        write_arrived(&inbox.join("unknown-arrival.pkg"), "pkg", 2000, None); // 받은 시각을 알 수 없다
        write_arrived(&inbox.join("really-old.pkg"), "pkg", 30, Some(30));
        let cfg = save_config_value(&json!({ "targets": [{ "id": "inbox", "path": inbox, "rules": [
            { "name": "오래된 설치 파일", "match": { "extensions": ["pkg"], "olderThanDays": 7 }, "action": "trash" }
        ]}]}))
        .unwrap();
        let scan = scan_target(&cfg, Some("inbox")).unwrap();
        let planned: Vec<&str> = scan.items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(planned, vec!["really-old.pkg"], "받은 지 7일 지난 것만. 방금 받은 옛 날짜 파일, 받은 시각을 모르는 파일은 그대로 둔다");
        assert_eq!(count_matching(&cfg, &cfg.targets[0]), 1);
        let r = rule_with(json!({ "olderThanDays": 7 }));
        let now = now_ms();
        let days = |d: f64| now - d * DAY_MS;
        assert!(match_rule(&r, "a", days(30.0), Some(days(30.0)), now));
        assert!(!match_rule(&r, "a", days(3000.0), Some(days(1.0)), now), "받은 지 하루");
        assert!(!match_rule(&r, "a", days(1.0), Some(days(30.0)), now), "어제 고침");
        assert!(!match_rule(&r, "a", days(3000.0), None, now), "받은 시각 모름 → 옮기지 않음");
    }

    #[test]
    fn rule_pointing_outside_is_blocked_in_preview() {
        let _g = guard();
        let sb = sandbox();
        let inbox = sb.join("outside-in");
        let _ = fs::remove_dir_all(&inbox);
        let _ = fs::remove_dir_all(sb.join("config"));
        write(&inbox.join("a.pdf"), "a", 0);
        write(&inbox.join("b.pdf"), "b", 0);
        write(&inbox.join("c.txt"), "c", 0);
        let cfg = save_config_value(&json!({ "targets": [{ "id": "inbox", "path": inbox, "rules": [
            { "name": "밖으로", "match": { "extensions": ["pdf"] }, "action": "move", "dest": sb.join("not-a-target") },
            { "name": "글", "match": { "extensions": ["txt", "pdf"] }, "action": "move", "dest": "글" }
        ]}]}))
        .unwrap();
        let scan = scan_target(&cfg, Some("inbox")).unwrap();
        assert_eq!(scan.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["c.txt"], "실행할 수 있는 것만 계획에 든다");
        assert_eq!(scan.blocked.len(), 2);
        assert!(scan.blocked.iter().all(|i| i.note.as_deref() == Some("outside")));
        assert_eq!(scan.blocked[0].also_matched, vec!["글"], "뒤에서 같이 맞는 규칙을 알려 준다");
        assert!(scan.unmatched.is_empty(), "규칙에는 걸린 파일이다");
        assert!(!rule_dest_inside(&cfg, &cfg.targets[0], &cfg.targets[0].rules[0]));
        assert!(rule_dest_inside(&cfg, &cfg.targets[0], &cfg.targets[0].rules[1]));
        let res = apply_plan(&cfg, scan.items, "test");
        assert!(res.results.iter().all(|r| r.ok), "미리보기에 나온 것은 실행에서 실패하지 않는다");
        assert!(!sb.join("not-a-target").exists());
    }

    #[test]
    fn name_race_never_overwrites() {
        let _g = guard();
        let (cfg, inbox) = simple("race-name", &["a.pdf"]);
        let item = scan_target(&cfg, Some("inbox")).unwrap().items.remove(0);
        // 계획을 다시 확인한 뒤, 옮기기 직전에 다른 프로그램이 같은 이름을 만든다
        let once = std::sync::atomic::AtomicBool::new(false);
        set_before_move(Some(Box::new(move |dest| {
            if !once.swap(true, std::sync::atomic::Ordering::SeqCst) {
                write(dest, "someone else", 0);
            }
        })));
        let res = apply_plan(&cfg, vec![item], "test");
        set_before_move(None);
        assert!(res.results[0].ok, "{}", res.results[0].message);
        assert_eq!(fs::read_to_string(inbox.join("문서").join("a.pdf")).unwrap(), "someone else", "끼어든 파일을 덮어쓰지 않는다");
        assert_eq!(fs::read_to_string(inbox.join("문서").join("a (2).pdf")).unwrap(), "a.pdf");
        let run = res.run.unwrap();
        assert_eq!(run.ops.len(), 1, "실패한 첫 시도는 기록에 남지 않는다");
        assert!(run.ops[0].to.as_deref().unwrap().ends_with("a (2).pdf"));
        assert_eq!(undo_last()["restored"], json!(1));
        assert!(inbox.join("문서").join("a.pdf").exists(), "되돌려도 남의 파일은 그대로");
    }

    #[test]
    fn links_packages_and_downloads_in_progress_are_left_alone() {
        let _g = guard();
        let sb = sandbox();
        let inbox = sb.join("leave-alone");
        let _ = fs::remove_dir_all(&inbox);
        let _ = fs::remove_dir_all(sb.join("config"));
        write(&inbox.join("real.pdf"), "real", 0);
        write(&inbox.join("movie.pdf.crdownload"), "chrome", 0);
        write(&inbox.join("movie.pdf.part"), "firefox", 0);
        write(&inbox.join("movie.pdf.download"), "safari file", 0);
        write(&inbox.join("big.pdf.download").join("big.pdf"), "safari package", 0);
        write(&inbox.join(".smash-c_1.tmp"), "our temp", 0);
        write(&inbox.join("Tool.app").join("Contents").join("manual.pdf"), "inside app", 0);
        write(&inbox.join("Solution").join("App.sln"), "sln", 0);
        write(&inbox.join("Solution").join("spec.pdf"), "in project", 0);
        write(&inbox.join("plain").join("deep.pdf"), "deep", 0);
        write(&sb.join("leave-target.pdf"), "target of link", 0);
        #[cfg(unix)]
        std::os::unix::fs::symlink(sb.join("leave-target.pdf"), inbox.join("link.pdf")).unwrap();
        let cfg = save_config_value(&json!({
            "targets": [{ "id": "inbox", "path": inbox, "recursive": true, "rules": [
                { "name": "문서", "match": { "extensions": ["pdf", "download", "crdownload", "part", "tmp", "sln"] }, "action": "move", "dest": "문서" }
            ]}],
            "skip": { "hidden": false, "patterns": ["*.crdownload", "*.download", "*.part"] }
        }))
        .unwrap();
        let scan = scan_target(&cfg, Some("inbox")).unwrap();
        let mut names: Vec<&str> = scan.items.iter().map(|i| i.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["deep.pdf", "real.pdf"], "바로가기·앱 꾸러미·개발 프로젝트·받는 중 파일·임시 파일은 계획에 없다");
        assert!(scan.unmatched.is_empty());
        // 계획을 꾸며 넣어도 실행 단계가 막는다
        let s = |p: PathBuf| p.to_string_lossy().into_owned();
        let forged = |src: PathBuf| Item { id: "x".into(), src: s(src.clone()), action: "move".into(), dest: s(inbox.join("문서").join(src.file_name().unwrap())), ..Default::default() };
        let res = apply_plan(&cfg, vec![forged(inbox.join("link.pdf")), forged(inbox.join("Tool.app")), forged(inbox.join("big.pdf.download"))], "test");
        assert!(res.results.iter().all(|r| !r.ok), "{:?}", res.results);
        assert!(res.run.as_ref().is_some_and(|r| r.ops.is_empty() && r.failed.len() == 3), "실패는 기록에 남는다");
        #[cfg(unix)]
        assert!(fs::symlink_metadata(inbox.join("link.pdf")).unwrap().file_type().is_symlink());
        assert!(inbox.join("Tool.app").join("Contents").join("manual.pdf").exists());
        assert!(sb.join("leave-target.pdf").exists());
        assert!(inbox.join("big.pdf.download").join("big.pdf").exists());
    }

    #[test]
    fn deep_trees_report_truncation() {
        let _g = guard();
        let sb = sandbox();
        let inbox = sb.join("deep");
        let _ = fs::remove_dir_all(&inbox);
        let _ = fs::remove_dir_all(sb.join("config"));
        let mut dir = inbox.clone();
        for i in 0..MAX_DEPTH + 2 {
            dir = dir.join(format!("d{i}"));
        }
        write(&dir.join("buried.pdf"), "x", 0);
        write(&inbox.join("top.pdf"), "x", 0);
        let cfg = save_config_value(&json!({ "targets": [{ "id": "inbox", "path": inbox, "recursive": true, "rules": [
            { "name": "문서", "match": { "extensions": ["pdf"] }, "action": "move", "dest": "문서" }
        ]}]}))
        .unwrap();
        let scan = scan_target(&cfg, Some("inbox")).unwrap();
        assert!(scan.truncated, "깊이 상한에 걸리면 다 못 봤다고 알린다");
        assert_eq!(scan.items.len(), 1);
        let mut flat = cfg.clone();
        flat.targets[0].recursive = false;
        assert!(!scan_target(&flat, Some("inbox")).unwrap().truncated);
    }

    #[test]
    fn nothing_moves_when_history_cannot_be_written() {
        let _g = guard();
        let (cfg, inbox) = simple("no-journal", &["a.pdf", "b.pdf"]);
        // 기록 폴더 자리에 파일이 있어 기록을 만들 수 없다
        fs::write(crate::history::history_dir(), "not a folder").unwrap();
        let res = apply_plan(&cfg, scan_target(&cfg, Some("inbox")).unwrap().items, "test");
        assert!(res.results.iter().all(|r| !r.ok), "{:?}", res.results);
        assert!(res.history_error.is_some());
        assert!(inbox.join("a.pdf").exists() && inbox.join("b.pdf").exists(), "되돌릴 수 없는 이동은 하지 않는다");
        fs::remove_file(crate::history::history_dir()).unwrap();
    }

    #[test]
    fn folder_shortcut_cannot_lead_claude_outside() {
        let _g = guard();
        let (cfg, inbox) = simple("escape", &["a.pdf"]);
        let outside = sandbox().join("escape-outside");
        let _ = fs::remove_dir_all(&outside);
        write(&outside.join("secret.pdf"), "s", 0);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, inbox.join("shortcut")).unwrap();
            let s = |p: PathBuf| p.to_string_lossy().into_owned();
            let planned = plan_from_moves(
                &cfg,
                vec![
                    MoveReq { src: s(inbox.join("a.pdf")), dest_folder: Some(s(inbox.join("shortcut").join("in"))), action: "move".into(), reason: String::new() },
                    MoveReq { src: s(inbox.join("shortcut").join("secret.pdf")), dest_folder: Some(s(inbox.join("문서"))), action: "move".into(), reason: String::new() },
                    MoveReq { src: s(inbox.join("a.pdf")), dest_folder: Some(s(inbox.join("새 폴더").join("하위"))), action: "move".into(), reason: String::new() },
                ],
            );
            assert_eq!(planned.rejected.len(), 2, "바로가기를 거쳐 바깥으로 나가거나 바깥 파일을 들이는 이동은 거절");
            assert_eq!(planned.items.len(), 1, "아직 없는 폴더는 있는 데까지만 확인한다");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn where_from_condition() {
        let _g = guard();
        let sb = sandbox();
        let inbox = sb.join("from");
        let _ = fs::remove_dir_all(&inbox);
        let _ = fs::remove_dir_all(sb.join("config"));
        write(&inbox.join("repo.zip"), "z", 0);
        write(&inbox.join("other.zip"), "z", 0);
        let tagged = std::process::Command::new("xattr").args(["-w", "com.apple.metadata:kMDItemWhereFroms", "https://github.com/x/y/archive/main.zip"]).arg(inbox.join("repo.zip")).status().is_ok_and(|s| s.success());
        if !tagged {
            return;
        }
        let cfg = save_config_value(&json!({ "targets": [{ "id": "inbox", "path": inbox, "rules": [
            { "name": "깃허브", "match": { "fromIncludes": ["GitHub.com"] }, "action": "move", "dest": "개발리소스" }
        ]}]}))
        .unwrap();
        let scan = scan_target(&cfg, Some("inbox")).unwrap();
        assert_eq!(scan.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["repo.zip"]);
        assert_eq!(scan.unmatched.len(), 1, "받은 곳 기록이 없는 파일은 맞지 않는다");
    }

    /// 기록을 파일마다 디스크에 내려쓰는 비용. `cargo test -- --ignored --nocapture journal_cost`
    #[test]
    #[ignore]
    fn journal_cost() {
        let _g = guard();
        let names: Vec<String> = (0..300).map(|i| format!("f{i}.pdf")).collect();
        let (cfg, _inbox) = simple("cost", &names.iter().map(String::as_str).collect::<Vec<_>>());
        let items = scan_target(&cfg, Some("inbox")).unwrap().items;
        let t = std::time::Instant::now();
        let res = apply_plan(&cfg, items, "test");
        let took = t.elapsed();
        assert!(res.results.iter().all(|r| r.ok));
        println!("300개 옮기기 {:?} (파일당 {:.2}ms)", took, took.as_secs_f64() * 1000.0 / 300.0);
        let t = std::time::Instant::now();
        assert_eq!(undo_last()["restored"], json!(300));
        println!("300개 되돌리기 {:?}", t.elapsed());
    }

    #[test]
    fn apply_renames_if_dest_appears() {
        let _g = guard();
        let (cfg, inbox, _) = setup();
        let item = scan_target(&cfg, Some("inbox")).unwrap().items.into_iter().find(|i| i.name == "photo.JPG").unwrap();
        write(&inbox.join("이미지").join("photo.JPG"), "someone else", 0);
        let applied = apply_plan(&cfg, vec![item], "test");
        assert!(applied.results[0].ok);
        assert_eq!(fs::read_to_string(inbox.join("이미지").join("photo.JPG")).unwrap(), "someone else");
        assert!(inbox.join("이미지").join("photo (2).JPG").exists());
    }

    #[test]
    fn apply_duplicate_without_original_moves_back() {
        let _g = guard();
        let (cfg, inbox, _) = setup();
        let dup = scan_target(&cfg, Some("inbox")).unwrap().items.into_iter().find(|i| i.name == "same.pdf").unwrap();
        assert_eq!(dup.note.as_deref(), Some("duplicate"));
        fs::remove_file(inbox.join("문서").join("same.pdf")).unwrap();
        let applied = apply_plan(&cfg, vec![dup], "test");
        assert!(applied.results[0].ok, "{}", applied.results[0].message);
        assert_eq!(fs::read_to_string(inbox.join("문서").join("same.pdf")).unwrap(), "identical");
    }

    #[test]
    fn rejects_outside_roots() {
        let _g = guard();
        let (cfg, inbox, archive) = setup();
        let outside = sandbox().join("outside.txt");
        write(&outside, "x", 0);
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let mv = |src: &Path, dest: &str| MoveReq { src: s(src), dest_folder: Some(dest.into()), action: "move".into(), reason: String::new() };
        let planned = plan_from_moves(
            &cfg,
            vec![
                mv(&outside, &s(&archive)),
                mv(&inbox.join("mystery.bin"), &s(&crate::config::home())),
                mv(&inbox.join("mystery.bin"), "기타"),
            ],
        );
        assert_eq!(planned.rejected.len(), 2);
        assert_eq!(planned.items.len(), 1);
        assert_eq!(planned.items[0].dest, s(&inbox.join("기타").join("mystery.bin")));
        let forged = Item { id: "x".into(), src: s(&outside), action: "move".into(), dest: s(&archive.join("outside.txt")), ..Default::default() };
        let res = apply_plan(&cfg, vec![forged], "app");
        assert!(!res.results[0].ok);
        assert!(outside.exists());
    }

    #[test]
    fn trash_keeps_original() {
        let _g = guard();
        let (cfg, inbox, _) = setup(); // 시험에서는 임시 휴지통(SMASH_TRASH_DIR)으로 간다
        let item = scan_target(&cfg, Some("inbox")).unwrap().items.into_iter().find(|i| i.name == "same.pdf").unwrap();
        let res = apply_plan(&cfg, vec![item], "test");
        assert!(res.results[0].ok, "{}", res.results[0].message);
        assert!(!inbox.join("same.pdf").exists());
        assert!(inbox.join("문서").join("same.pdf").exists(), "원본은 남아 있다");
    }

    #[test]
    fn rule_preview_and_missing_dirs() {
        let _g = guard();
        let (cfg, inbox, _) = setup();
        let mut got = test_rule(&cfg, "inbox", &json!({ "match": { "extensions": ["pdf"] } })).unwrap();
        got.sort();
        let mut want = vec!["report.pdf".to_string(), "same.pdf".into(), "연봉 계약서.pdf".into()];
        want.sort();
        assert_eq!(got, want);
        assert!(test_rule(&cfg, "inbox", &json!({ "match": {} })).unwrap().is_empty(), "조건 없으면 아무것도 안 잡힌다");
        let dest = inbox.join("새폴더").join("하위").join("a.txt");
        assert_eq!(missing_dirs(&dest), vec![inbox.join("새폴더").join("하위"), inbox.join("새폴더")]);
        assert!(missing_dirs(&inbox.join("문서").join("a.txt")).is_empty());
        let _ = load_config();
    }

    #[test]
    fn linked_recursive_grouped_scan() {
        let _g = guard();
        let sb = sandbox();
        let docs = sb.join("docs2");
        let dl = sb.join("dl2");
        let _ = fs::remove_dir_all(&docs);
        let _ = fs::remove_dir_all(&dl);
        write(&docs.join("영수증_11월.pdf"), "r", 0);
        write(&docs.join("내 폴더").join("메모.pdf"), "m", 0);
        write(&docs.join("proj").join("package.json"), "{}", 0);
        write(&docs.join("proj").join("spec.pdf"), "s", 0);
        write(&docs.join("문서").join("이미 정리.pdf"), "x", 0);
        write(&docs.join("문서").join("2025").join("old.pdf"), "o", 0);
        write(&docs.join("문서").join("2024 회사").join("영수증_10월.pdf"), "r", 0);
        write(&dl.join("받은 계약서.pdf"), "c", 0);
        let cfg = save_config_value(&json!({ "targets": [
            { "id": "docs", "path": docs, "recursive": true, "rules": [
                { "name": "문서", "match": { "extensions": ["pdf"] }, "action": "move", "dest": "문서", "group": "auto" }
            ]},
            { "id": "dl", "path": dl, "link": "docs", "rules": [] }
        ]}))
        .unwrap();
        let s = |p: PathBuf| p.to_string_lossy().into_owned();
        let scan = scan_target(&cfg, Some("docs")).unwrap();
        let by = |n: &str| scan.items.iter().find(|i| i.name == n).cloned();
        assert_eq!(by("영수증_11월.pdf").unwrap().dest, s(docs.join("문서").join("영수증·청구서").join("영수증_11월.pdf")));
        assert_eq!(by("메모.pdf").unwrap().dest, s(docs.join("문서").join("메모.pdf")), "하위 폴더 파일도 정리");
        assert!(by("spec.pdf").is_none(), "개발 프로젝트 폴더 안은 안 들어간다");
        assert!(by("이미 정리.pdf").is_none() && by("old.pdf").is_none(), "이미 대분류 안에 있고 소분류가 없으면 그대로");
        assert!(by("영수증_10월.pdf").is_none(), "대분류 안 사용자 하위 폴더는 소분류가 붙어도 풀지 않는다");
        assert!(scan_target_with(&cfg, Some("docs"), false).unwrap().items.iter().all(|i| i.name != "메모.pdf"), "자동 정리는 하위 폴더를 안 본다");
        assert_eq!(by("영수증_11월.pdf").unwrap().group_base, Some(s(docs.join("문서"))));

        let scan = scan_target(&cfg, Some("dl")).unwrap();
        assert_eq!(scan.items[0].dest, s(docs.join("문서").join("계약서").join("받은 계약서.pdf")), "연결된 폴더는 기준 폴더 규칙·트리로");

        let mut flat = cfg.clone();
        flat.targets[0].recursive = false;
        assert!(scan_target(&flat, Some("docs")).unwrap().items.iter().all(|i| i.name != "메모.pdf"), "꺼져 있으면 사용자 폴더는 안 건드린다");

        // 이미지 내용 묶기 결과로 다시 계획
        let items = scan_target(&cfg, Some("docs")).unwrap().items;
        let id = items.iter().find(|i| i.name == "메모.pdf").unwrap().id.clone();
        let assign: HashMap<String, String> = [(id.clone(), "게임/엘든 링".to_string())].into();
        let re = regroup(&cfg, items, &assign);
        assert_eq!(re.iter().find(|i| i.id == id).unwrap().dest, s(docs.join("문서").join("게임").join("엘든 링").join("메모.pdf")));
    }

    #[test]
    fn split_names() {
        assert_eq!(split_name("a.tar.gz"), ("a.tar".into(), ".gz".into()));
        assert_eq!(split_name(".bashrc"), (".bashrc".into(), String::new()));
        assert_eq!(split_name("noext"), ("noext".into(), String::new()));
    }
}
