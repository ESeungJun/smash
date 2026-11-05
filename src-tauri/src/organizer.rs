//! 폴더 스캔 → 정리 계획 → 실행 → 기록·되돌리기.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use crate::config::{
    abs, clean, contract_home, expand_home, history_path, new_id, now_ms, resolve, ser_num, write_json_atomic, Config, Rule,
    Target,
};
use crate::lang::l;
use crate::trash::move_to_trash;

const DAY_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;
const HISTORY_LIMIT: usize = 50;
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
    "settings.gradle", "Gemfile", "composer.json", "CMakeLists.txt", "Makefile", ".project", ".sln", "Podfile", "pubspec.yaml",
];
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
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub target_id: String,
    pub root: String,
    pub items: Vec<Item>,
    pub unmatched: Vec<FileInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Op {
    #[serde(rename = "type")]
    pub kind: String,
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Run {
    pub id: String,
    pub at: String,
    pub source: String,
    pub ops: Vec<Op>,
    pub undone: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ApplyResult {
    pub id: String,
    pub name: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Serialize, Debug)]
pub struct Applied {
    pub results: Vec<ApplyResult>,
    pub run: Option<Run>,
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
        Skipper { hidden: cfg.skip.hidden, patterns: cfg.skip.patterns.iter().map(|p| lower_nfc(p)).collect() }
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
    m.extensions.len() + m.name_includes.len() > 0 || m.older_than_days > 0.0
}

pub fn match_rule(rule: &Rule, name: &str, mtime_ms: f64, now: f64) -> bool {
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
    if m.older_than_days > 0.0 && now - mtime_ms < m.older_than_days * DAY_MS {
        return false;
    }
    true
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
        if !candidate.exists() && !reserved.contains(&candidate) {
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
    if dest.exists() {
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
        out.push(FileInfo { name, path: entry.path().to_string_lossy().into_owned(), size: meta.len() as f64, mtime_ms: mtime_ms(&meta) });
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
}

/// 정리 대상의 파일들. 하위 폴더까지 정리가 꺼져 있으면 바로 아래 파일만.
/// 켜져 있어도 숨김·건너뛰기 폴더, 앱·라이브러리 꾸러미, 개발 프로젝트, 다른 정리 대상 폴더 안에는 들어가지 않는다.
pub fn scan_files(cfg: &Config, target: &Target) -> Result<Vec<FileInfo>, String> {
    let root = target_root(target);
    let skip = Skipper::new(cfg);
    let mut out = list_files(&root, &skip)?;
    if !target.recursive {
        return Ok(out);
    }
    let others: Vec<PathBuf> = target_roots(cfg).into_iter().filter(|r| *r != root).collect();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.clone(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        if depth >= MAX_DEPTH || out.len() >= MAX_SCAN_FILES {
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
                break;
            }
            if let Ok(files) = list_files(&sub, &skip) {
                out.extend(files.into_iter().take(room));
            }
            stack.push((sub, depth + 1));
        }
    }
    out.truncate(MAX_SCAN_FILES);
    Ok(out)
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
    let files = scan_files(cfg, target)?;

    // 1. 파일마다 처음 맞는 규칙
    let mut hit: Vec<Option<(&Rule, PathBuf)>> = Vec::with_capacity(files.len());
    let mut unmatched = Vec::new();
    for file in &files {
        match rules.iter().find(|(r, _)| match_rule(r, &file.name, file.mtime_ms, now)) {
            Some((rule, base)) => {
                let dir = if rule.action == "trash" { PathBuf::new() } else { clean(&resolve_dest_dir(base, &rule.dest)) };
                hit.push(Some((rule, dir)));
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
        if let Some((rule, dir)) = h {
            if rule.action == "move" && !rule.group.is_empty() {
                groups.entry((rule.id.clone(), dir.clone())).or_default().push(i);
            }
        }
    }
    let mut sub: Vec<Option<String>> = vec![None; files.len()];
    for ((_, dir), idx) in &groups {
        let mode = hit[idx[0]].as_ref().map(|(r, _)| r.group.as_str()).unwrap_or("");
        let gfiles: Vec<crate::grouping::GFile> =
            idx.iter().map(|&i| crate::grouping::GFile { name: files[i].name.clone(), mtime_ms: files[i].mtime_ms }).collect();
        for (k, s) in crate::grouping::subfolders(mode, &gfiles, &existing_subdirs(dir, &skip)).into_iter().enumerate() {
            sub[idx[k]] = s.and_then(|s| crate::grouping::clean_folder(&s));
        }
    }

    // 3. 파일 순서대로 계획 (같은 이름 충돌을 순서대로 막는다)
    let mut reserved = HashSet::new();
    let mut items = Vec::new();
    for (i, file) in files.iter().enumerate() {
        let Some((rule, dir)) = &hit[i] else { continue };
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
            plan_move(src, &dest_dir, &mut reserved)
        };
        if let Some(p) = planned {
            let mut item = make_item(file, p, Some(rule.id.clone()), rule.name.clone());
            if rule.action == "move" {
                item.group_base = Some(dir.to_string_lossy().into_owned());
            }
            items.push(item);
        }
    }
    Ok(Scan { target_id: target.id.clone(), root: root.to_string_lossy().into_owned(), items, unmatched })
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
        .filter(|f| match_rule(&candidate, &f.name, f.mtime_ms, now))
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
        let file = FileInfo {
            name: src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            path: src.to_string_lossy().into_owned(),
            size: st.len() as f64,
            mtime_ms: mtime_ms(&st),
        };
        if m.action == "trash" {
            items.push(make_item(&file, Plan { action: "trash", dest: String::new(), note: None, duplicate_of: None }, None, m.reason));
            continue;
        }
        let base = src.parent().map(Path::to_path_buf).unwrap_or_default();
        let dest_dir = resolve_dest_dir(&base, m.dest_folder.as_deref().unwrap_or(""));
        if !inside_any_root(cfg, &dest_dir) {
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

fn is_cross_device(e: &std::io::Error) -> bool {
    // EXDEV(18, 유닉스) · ERROR_NOT_SAME_DEVICE(17, 윈도우)
    matches!(e.raw_os_error(), Some(18) if cfg!(unix)) || matches!(e.raw_os_error(), Some(17) if cfg!(windows))
}

fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => {
            // 다른 디스크로: 덮어쓰지 않게 복사하고 수정 시간을 맞춘 뒤 원본을 지운다
            let meta = fs::metadata(from).map_err(|e| e.to_string())?;
            let mut src = fs::File::open(from).map_err(|e| e.to_string())?;
            let mut dst = fs::OpenOptions::new().write(true).create_new(true).open(to).map_err(|e| e.to_string())?;
            std::io::copy(&mut src, &mut dst).map_err(|e| e.to_string())?;
            drop(dst);
            let _ = filetime::set_file_times(
                to,
                filetime::FileTime::from_last_access_time(&meta),
                filetime::FileTime::from_last_modification_time(&meta),
            );
            fs::remove_file(from).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

pub fn load_history() -> Vec<Run> {
    let Ok(text) = fs::read_to_string(history_path()) else { return vec![] };
    match serde_json::from_str::<Vec<Value>>(&text) {
        Ok(list) => list.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect(),
        Err(_) => vec![],
    }
}

fn save_history(runs: &[Run]) {
    let start = runs.len().saturating_sub(HISTORY_LIMIT);
    let _ = write_json_atomic(&history_path(), &runs[start..].to_vec());
}

pub fn apply_plan(cfg: &Config, items: Vec<Item>, source: &str) -> Applied {
    let mut ops = Vec::new();
    let mut results = Vec::new();
    let mut reserved = HashSet::new();
    for mut item in items {
        let src = abs(&item.src);
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let outcome: Result<String, String> = (|| {
            if !inside_any_root(cfg, &src) {
                return Err(l("정리 대상 폴더 밖의 파일입니다", "The file is outside the folders you set up"));
            }
            let st = fs::symlink_metadata(&src).map_err(|e| e.to_string())?;
            if !st.is_file() {
                return Err(l("파일이 아닙니다", "Not a file"));
            }
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
            let from = src.to_string_lossy().into_owned();
            if item.action == "trash" {
                move_to_trash(&src)?;
                ops.push(Op { kind: "trash".into(), from, to: None });
                return Ok(l("휴지통으로 보냄", "Moved to Trash"));
            }
            let wanted = abs(&item.dest);
            if !inside_any_root(cfg, &wanted) {
                return Err(l("목적지가 정리 대상 폴더 밖입니다", "The destination is outside the folders you set up"));
            }
            // 계획 이후에 상황이 바뀌었을 수 있어 실행 직전에 다시 확인한다.
            let dir = wanted.parent().map(Path::to_path_buf).unwrap_or_default();
            let planned = plan_move(&src, &dir, &mut reserved).ok_or_else(|| l("이미 그 폴더에 있습니다", "Already in that folder"))?;
            if planned.action == "trash" {
                move_to_trash(&src)?;
                ops.push(Op { kind: "trash".into(), from, to: None });
                return Ok(l("같은 파일이 이미 있어 휴지통으로 보냄", "Same file already there, moved to Trash"));
            }
            move_file(&src, Path::new(&planned.dest))?;
            ops.push(Op { kind: "move".into(), from, to: Some(planned.dest.clone()) });
            Ok(format!("→ {}", contract_home(&planned.dest)))
        })();
        let (ok, message) = match outcome {
            Ok(m) => (true, m),
            Err(m) => (false, m),
        };
        results.push(ApplyResult { id: item.id.clone(), name, ok, message });
    }
    let run = if ops.is_empty() {
        None
    } else {
        let run = Run { id: new_id("run"), at: iso_now(), source: source.into(), ops, undone: false };
        let mut history = load_history();
        history.push(run.clone());
        save_history(&history);
        Some(run)
    };
    Applied { results, run }
}

pub fn iso_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

pub fn undo_last() -> Value {
    let mut runs = load_history();
    let Some(idx) = runs.iter().rposition(|r| !r.undone) else {
        return json!({ "ok": false, "message": l("되돌릴 작업이 없습니다", "Nothing to undo"), "restored": 0, "failed": [], "trashed": [] });
    };
    let mut restored = 0;
    let mut failed = Vec::new();
    for op in runs[idx].ops.iter().rev() {
        if op.kind != "move" {
            continue;
        }
        let to = op.to.clone().unwrap_or_default();
        let res = (|| {
            if !Path::new(&to).exists() {
                return Err(l("옮긴 파일이 없습니다", "The moved file is gone"));
            }
            if Path::new(&op.from).exists() {
                return Err(l("원래 자리에 같은 이름 파일이 있습니다", "A file with the same name is in the original place"));
            }
            move_file(Path::new(&to), Path::new(&op.from))
        })();
        match res {
            Ok(()) => restored += 1,
            Err(reason) => failed.push(json!({ "file": contract_home(&to), "reason": reason })),
        }
    }
    runs[idx].undone = true;
    let run_id = runs[idx].id.clone();
    let trashed: Vec<String> = runs[idx].ops.iter().filter(|o| o.kind == "trash").map(|o| contract_home(&o.from)).collect();
    save_history(&runs);
    json!({ "ok": true, "runId": run_id, "restored": restored, "failed": failed, "trashed": trashed })
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
    use crate::test_util::{sandbox, write, LOCK};

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
        assert!(!match_rule(&rule_with(json!({})), "a.txt", 0.0, now_ms()));
    }

    #[test]
    fn nfd_korean_name_matches() {
        let name: String = "스크린샷 2026-11-01 오후 9.20.11.png".nfd().collect();
        assert!(match_rule(&rule_with(json!({ "extensions": ["png"], "nameIncludes": ["스크린샷"] })), &name, 0.0, now_ms()));
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
        if cfg!(target_os = "macos") && std::env::var("CI").is_err() {
            return; // 로컬 맥에서는 실제 휴지통을 건드리지 않는다
        }
        let (cfg, inbox, _) = setup();
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
