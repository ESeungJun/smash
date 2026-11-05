//! 사용자 PC 에 로그인된 Claude 와 연동한다. 앱은 API 키를 갖지 않는다.
//!  - claude CLI(`claude -p`)로 분류 제안·말로 규칙 만들기
//!  - 이 앱의 MCP 서버를 Claude Code / Claude Desktop 에 등록하기
//!  - Electron 시절 등록(예전 실행 경로)을 새 실행 파일로 자동 교체하기

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::config::{contract_home, expand_home, home, write_json_atomic, Config};
use crate::lang::l;
use crate::organizer::{folder_tree, plan_from_moves, FileInfo, MoveReq};

pub const MCP_NAME: &str = "smash";
/// 예전 이름으로 등록된 서버는 다시 등록할 때 지운다.
const LEGACY_MCP_NAMES: [&str; 2] = ["folder-tidy", "deungjjak"];
const MAX_FILES_PER_REQUEST: usize = 200;
const SUGGEST_TIMEOUT: Duration = Duration::from_secs(180);

/// Claude 가 이 앱을 MCP 서버로 띄우는 명령: 앱 실행 파일 + `--mcp`
#[derive(Clone, Debug)]
pub struct Launch {
    pub command: String,
    pub args: Vec<String>,
}

impl Launch {
    pub fn current() -> Self {
        let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        Launch { command: exe, args: vec!["--mcp".into()] }
    }
    pub fn to_json(&self) -> Value {
        json!({ "command": self.command, "args": self.args, "env": {} })
    }
}

fn suggestion_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "suggestions": { "type": "array", "items": { "type": "object",
            "properties": { "file": { "type": "string" }, "dest": { "type": ["string", "null"] }, "reason": { "type": "string" } },
            "required": ["file", "dest", "reason"] } } },
        "required": ["suggestions"]
    })
}

fn rule_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "extensions": { "type": "array", "items": { "type": "string" } },
            "nameIncludes": { "type": "array", "items": { "type": "string" } },
            "olderThanDays": { "type": "number" },
            "action": { "type": "string", "enum": ["move", "trash"] },
            "dest": { "type": "string" }
        },
        "required": ["name", "extensions", "nameIncludes", "olderThanDays", "action", "dest"]
    })
}

// ── CLI 찾기·실행 ─────────────────────────────────────

fn candidate_paths() -> Vec<PathBuf> {
    let h = home();
    if cfg!(target_os = "windows") {
        let app_data = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| h.join("AppData").join("Roaming"));
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| h.join("AppData").join("Local"));
        vec![
            h.join(".local").join("bin").join("claude.exe"),
            local.join("Programs").join("claude").join("claude.exe"),
            app_data.join("npm").join("claude.cmd"),
        ]
    } else {
        vec![
            h.join(".local").join("bin").join("claude"),
            h.join(".claude").join("local").join("claude"),
            PathBuf::from("/opt/homebrew/bin/claude"),
            PathBuf::from("/usr/local/bin/claude"),
        ]
    }
}

static CACHED_CLI: Mutex<Option<PathBuf>> = Mutex::new(None);

fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

pub fn find_claude_cli() -> Option<PathBuf> {
    let mut cache = CACHED_CLI.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = cache.as_ref().filter(|p| p.exists()) {
        return Some(p.clone());
    }
    let mut found = candidate_paths().into_iter().find(|p| p.exists());
    if found.is_none() {
        // GUI 앱은 셸 PATH 를 모르기 때문에 로그인 셸에 한 번 물어본다.
        let out = if cfg!(target_os = "windows") {
            hidden(Command::new("where.exe").arg("claude")).output()
        } else {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
            Command::new(shell).args(["-lc", "command -v claude"]).stdin(Stdio::null()).output()
        };
        if let Ok(out) = out {
            // 셸 프로필이 인사말 등을 찍을 수 있어 실제로 있는 경로만 고른다.
            found = String::from_utf8_lossy(&out.stdout)
                .lines()
                .map(|s| PathBuf::from(s.trim()))
                .find(|p| p.is_absolute() && p.exists());
        }
    }
    *cache = found.clone();
    found
}

fn is_cmd_script(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("cmd")).unwrap_or(false)
}

/// configDir: None 이면 지금 환경 그대로, Some(None) 이면 기본 위치(~/.claude), Some(Some(dir)) 이면 그 위치.
pub fn run_cli(args: &[String], input: Option<&str>, timeout: Duration, config_dir: Option<Option<&Path>>) -> Result<String, String> {
    let cli = find_claude_cli().ok_or_else(|| {
        l(
            "Claude Code CLI를 찾지 못했습니다. claude.com/claude-code 에서 설치 후 로그인해 주세요.",
            "Claude Code CLI not found. Install it from claude.com/claude-code and sign in.",
        )
    })?;
    let mut cmd;
    if is_cmd_script(&cli) {
        // npm 으로 설치한 Windows 의 claude.cmd 는 cmd 를 거쳐야 실행된다.
        cmd = Command::new("cmd");
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            let quote = |a: &str| {
                if a.is_empty() || a.chars().any(|c| " \t\"&|<>^%".contains(c)) {
                    format!("\"{}\"", a.replace('"', "\"\""))
                } else {
                    a.to_string()
                }
            };
            let line: Vec<String> = std::iter::once(quote(&cli.to_string_lossy())).chain(args.iter().map(|a| quote(a))).collect();
            cmd.raw_arg(format!("/S /C \"{}\"", line.join(" ")));
        }
    } else {
        cmd = Command::new(&cli);
        cmd.args(args);
    }
    cmd.current_dir(std::env::temp_dir()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    match config_dir {
        Some(None) => {
            cmd.env_remove("CLAUDE_CONFIG_DIR");
        }
        Some(Some(dir)) => {
            cmd.env("CLAUDE_CONFIG_DIR", dir);
        }
        None => {}
    }
    hidden(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    // 입력이 없어도 stdin 을 닫아야 CLI 가 입력을 기다리지 않는다.
    if let Some(mut stdin) = child.stdin.take() {
        if let Some(text) = input {
            let _ = stdin.write_all(text.as_bytes());
        }
    }
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stdout.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break st;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(l("Claude 응답 시간이 초과됐습니다", "Claude took too long to answer"));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = out_t.join().unwrap_or_default();
    let err = err_t.join().unwrap_or_default();
    if status.success() {
        return Ok(out);
    }
    let msg: String = if err.trim().is_empty() { out.trim() } else { err.trim() }.chars().take(500).collect();
    Err(if msg.is_empty() {
        format!("{}{}", l("claude 종료 코드 ", "claude exited with code "), status.code().unwrap_or(-1))
    } else {
        msg
    })
}

pub fn cli_status() -> Value {
    let Some(cli) = find_claude_cli() else { return json!({ "installed": false }) };
    let path = contract_home(&cli.to_string_lossy());
    match run_cli(&["--version".into()], None, Duration::from_secs(10), None) {
        Ok(v) => json!({ "installed": true, "path": path, "version": v.trim() }),
        Err(e) => json!({ "installed": true, "path": path, "error": e }),
    }
}

// ── 분류 제안 ─────────────────────────────────────────

fn fmt_size(n: f64) -> String {
    let mb = (1u64 << 20) as f64;
    if n >= mb {
        format!("{:.1}MB", n / mb)
    } else {
        format!("{}KB", ((n / 1024.0).round() as i64).max(1))
    }
}

fn fmt_date(ms: f64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

pub fn build_prompt(target_root: &str, files: &[FileInfo], folders: &[String]) -> String {
    let mut lines = vec![
        "너는 파일 정리 도우미야. 아래 파일 각각을 가장 알맞은 폴더로 분류해 줘.".to_string(),
        "규칙:".into(),
        "- dest에는 \"폴더 목록\"에 있는 경로를 그대로 쓴다. 꼭 필요할 때만 정리 대상 폴더 바로 아래 새 폴더를 만든다(예: ~/Downloads/새폴더).".into(),
        "- 어디가 맞는지 확신이 없으면 dest를 null로 둔다.".into(),
        "- 삭제는 제안하지 않는다.".into(),
        l("- reason은 한국어로 30자 이내.", "- reason은 영어로 40자 이내."),
        "- file에는 파일 이름을 그대로 쓴다.".into(),
        String::new(),
        format!("지금 정리하는 폴더: {}", contract_home(target_root)),
        String::new(),
        "폴더 목록:".into(),
    ];
    lines.extend(folders.iter().map(|f| format!("- {}", contract_home(f))));
    lines.push(String::new());
    lines.push("파일 목록 (이름 | 크기 | 수정일):".into());
    lines.extend(files.iter().map(|f| format!("- {} | {} | {}", f.name, fmt_size(f.size), fmt_date(f.mtime_ms))));
    lines.push(String::new());
    lines.push("응답은 {\"suggestions\":[{\"file\":\"...\",\"dest\":\"...\" 또는 null,\"reason\":\"...\"}]} 형식의 JSON만.".into());
    lines.join("\n")
}

fn json_in_text(text: &str) -> Result<Value, String> {
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Err(l("Claude 응답을 해석하지 못했습니다", "Could not read Claude's answer"));
    };
    if end < start {
        return Err(l("Claude 응답을 해석하지 못했습니다", "Could not read Claude's answer"));
    }
    serde_json::from_str(&text[start..=end]).map_err(|_| l("Claude 응답을 해석하지 못했습니다", "Could not read Claude's answer"))
}

fn check_outer(stdout: &str) -> Result<Value, String> {
    let outer: Value = serde_json::from_str(stdout).map_err(|e| e.to_string())?;
    if outer.get("is_error").and_then(Value::as_bool) == Some(true) {
        return Err(outer.get("result").and_then(Value::as_str).map(String::from).unwrap_or_else(|| l("Claude 오류", "Claude error")));
    }
    Ok(outer)
}

pub fn parse_suggestions(stdout: &str) -> Result<Vec<Value>, String> {
    let outer = check_outer(stdout)?;
    if let Some(list) = outer.pointer("/structured_output/suggestions").and_then(Value::as_array) {
        return Ok(list.clone());
    }
    let text = outer.get("result").and_then(Value::as_str).unwrap_or("");
    Ok(json_in_text(text)?.get("suggestions").and_then(Value::as_array).cloned().unwrap_or_default())
}

fn base_args(cfg: &Config, schema: Value) -> Vec<String> {
    let mut args: Vec<String> =
        ["-p", "--output-format", "json", "--tools", "", "--no-session-persistence", "--strict-mcp-config", "--setting-sources", ""]
            .iter()
            .map(|s| s.to_string())
            .collect();
    if !cfg.ai.model.is_empty() {
        args.push("--model".into());
        args.push(cfg.ai.model.clone());
    }
    if !find_claude_cli().map(|p| is_cmd_script(&p)).unwrap_or(false) {
        args.push("--json-schema".into());
        args.push(schema.to_string());
    }
    args
}

pub fn suggest_placements(cfg: &Config, target_root: &str, unmatched: &[FileInfo]) -> Result<Value, String> {
    let files: Vec<FileInfo> = unmatched.iter().take(MAX_FILES_PER_REQUEST).cloned().collect();
    if files.is_empty() {
        return Ok(json!({ "items": [], "rejected": [], "noSuggestion": [] }));
    }
    let prompt = build_prompt(target_root, &files, &folder_tree(cfg, 2));
    let stdout = run_cli(&base_args(cfg, suggestion_schema()), Some(&prompt), SUGGEST_TIMEOUT, None)?;
    let mut remaining: Vec<&FileInfo> = files.iter().collect();
    let mut moves = Vec::new();
    let mut no_suggestion = Vec::new();
    for s in parse_suggestions(&stdout)? {
        let name = s.get("file").and_then(Value::as_str).unwrap_or("");
        let Some(pos) = remaining.iter().position(|f| f.name == name) else { continue };
        let file = remaining.remove(pos);
        match s.get("dest").and_then(Value::as_str).filter(|d| !d.is_empty()) {
            None => no_suggestion.push(file.name.clone()),
            Some(dest) => moves.push(MoveReq {
                src: file.path.clone(),
                dest_folder: Some(expand_home(dest)),
                action: "move".into(),
                reason: format!("AI: {}", s.get("reason").and_then(Value::as_str).unwrap_or("")).trim().to_string(),
            }),
        }
    }
    no_suggestion.extend(remaining.iter().map(|f| f.name.clone()));
    let plan = plan_from_moves(cfg, moves);
    Ok(json!({ "items": plan.items, "rejected": plan.rejected, "noSuggestion": no_suggestion }))
}

// ── 말로 규칙 만들기 ──────────────────────────────────

pub fn rule_from_text(cfg: &Config, target_root: &str, text: &str) -> Result<Value, String> {
    let folders: Vec<String> = folder_tree(cfg, 2).iter().map(|f| contract_home(f)).collect();
    let mut lines = vec![
        "사용자가 말로 설명한 파일 정리 규칙을 JSON으로 바꿔 줘.".to_string(),
        format!("정리 대상 폴더: {}", contract_home(target_root)),
        "- extensions: 점 없는 소문자 확장자 목록. 종류만 말하면 흔한 확장자를 채운다(예: 사진 → jpg, jpeg, png, heic, gif, webp).".into(),
        "- nameIncludes: 파일 이름에 들어 있어야 하는 단어. 없으면 빈 배열.".into(),
        "- olderThanDays: \"일주일 지난\" 같은 조건이 있으면 일 수, 없으면 0.".into(),
        "- action: 옮기면 move, 지우라고 하면 trash(실제로는 휴지통으로 간다).".into(),
        "- dest: move일 때 목적지. 아래 폴더 목록에 맞는 게 있으면 그 경로(~ 경로)를 그대로 쓰고, 없으면 정리 대상 폴더 기준 상대 경로로 새 폴더 이름을 쓴다. trash면 빈 문자열.".into(),
        l("- name: 한국어 규칙 이름, 15자 이내.", "- name: 영어 규칙 이름, 30자 이내."),
        String::new(),
        "폴더 목록:".into(),
    ];
    lines.extend(folders.iter().map(|f| format!("- {f}")));
    lines.push(String::new());
    lines.push(format!("사용자 설명: {text}"));
    let outer = check_outer(&run_cli(&base_args(cfg, rule_schema()), Some(&lines.join("\n")), SUGGEST_TIMEOUT, None)?)?;
    let rule = match outer.get("structured_output").filter(|v| v.is_object()) {
        Some(r) => r.clone(),
        None => json_in_text(outer.get("result").and_then(Value::as_str).unwrap_or(""))?,
    };
    let trash = rule.get("action").and_then(Value::as_str) == Some("trash");
    let m = crate::config::normalize_match(Some(&json!({
        "extensions": rule.get("extensions").cloned().unwrap_or(json!([])),
        "nameIncludes": rule.get("nameIncludes").cloned().unwrap_or(json!([])),
        "olderThanDays": rule.get("olderThanDays").cloned().unwrap_or(json!(0)),
    })));
    Ok(json!({
        "name": rule.get("name").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| l("새 규칙", "New rule")),
        "match": m,
        "action": if trash { "trash" } else { "move" },
        "dest": if trash { String::new() } else { rule.get("dest").and_then(Value::as_str).unwrap_or("").to_string() },
    }))
}

// ── MCP 등록 ──────────────────────────────────────────

pub struct Location {
    pub id: String,
    pub label: String,
    pub dir: Option<PathBuf>,
    pub file: PathBuf,
}

/// Claude Code 설정 위치들. 기본(~/.claude.json) + CLAUDE_CONFIG_DIR + Orca 앱 프로필.
pub fn claude_config_locations() -> Vec<Location> {
    let h = home();
    let mut locs = vec![Location { id: "default".into(), label: l("기본 (~/.claude.json)", "Default (~/.claude.json)"), dir: None, file: h.join(".claude.json") }];
    fn add(locs: &mut Vec<Location>, dir: PathBuf, label: String) {
        let resolved = crate::config::clean(&dir);
        if locs.iter().any(|l| l.dir.as_ref() == Some(&resolved)) {
            return;
        }
        locs.push(Location { id: resolved.to_string_lossy().into_owned(), label, file: resolved.join(".claude.json"), dir: Some(resolved) });
    }
    let orca = h.join("Library").join("Application Support").join("orca").join("claude-profiles");
    let selected = std::fs::read_to_string(orca.join("selected-host")).map(|s| s.trim().to_string()).unwrap_or_default();
    if let Ok(entries) = std::fs::read_dir(&orca) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            let dir = orca.join(&name).join("home");
            if !dir.join(".claude.json").exists() {
                continue;
            }
            let short: String = name.chars().take(8).collect();
            let in_use = !selected.is_empty() && crate::config::clean(Path::new(&selected)) == crate::config::clean(&dir);
            let label = l(&format!("Orca 프로필 {short}"), &format!("Orca profile {short}")) + &if in_use { l(" (사용 중)", " (in use)") } else { String::new() };
            add(&mut locs, dir, label);
        }
    }
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.is_empty() {
            let label = format!("CLAUDE_CONFIG_DIR ({})", contract_home(&dir));
            add(&mut locs, PathBuf::from(dir), label);
        }
    }
    locs
}

fn read_json(file: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

fn registered_entry(file: &Path) -> Option<Value> {
    read_json(file)?.get("mcpServers")?.get(MCP_NAME).cloned()
}

pub fn register_claude_code(launch: &Launch, location_ids: Option<Vec<String>>) -> Vec<Value> {
    let mut results = Vec::new();
    for loc in claude_config_locations() {
        if let Some(ids) = &location_ids {
            if !ids.contains(&loc.id) {
                continue;
            }
        }
        let dir = Some(loc.dir.as_deref());
        for name in std::iter::once(MCP_NAME).chain(LEGACY_MCP_NAMES) {
            let _ = run_cli(&["mcp".into(), "remove".into(), "-s".into(), "user".into(), name.into()], None, Duration::from_secs(30), dir);
        }
        let mut args: Vec<String> = vec!["mcp".into(), "add".into(), "-s".into(), "user".into(), MCP_NAME.into(), "--".into(), launch.command.clone()];
        args.extend(launch.args.iter().cloned());
        match run_cli(&args, None, Duration::from_secs(30), dir) {
            Ok(_) => results.push(json!({ "location": loc.label, "ok": registered_entry(&loc.file).is_some() })),
            Err(e) => results.push(json!({ "location": loc.label, "ok": false, "error": e })),
        }
    }
    results
}

pub fn claude_desktop_config_path() -> PathBuf {
    let h = home();
    if cfg!(target_os = "macos") {
        h.join("Library").join("Application Support").join("Claude").join("claude_desktop_config.json")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| h.join("AppData").join("Roaming")).join("Claude").join("claude_desktop_config.json")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| h.join(".config")).join("Claude").join("claude_desktop_config.json")
    }
}

pub fn register_claude_desktop(launch: &Launch) -> Result<String, String> {
    let file = claude_desktop_config_path();
    let mut data = if file.exists() {
        let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        let _ = std::fs::copy(&file, file.with_extension("json.bak"));
        serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())?
    } else {
        json!({})
    };
    set_server(&mut data, launch, true);
    write_json_atomic(&file, &data).map_err(|e| e.to_string())?;
    Ok(contract_home(&file.to_string_lossy()))
}

/// mcpServers.smash 를 이 앱으로 맞춘다. drop_legacy 면 예전 이름도 지운다.
fn set_server(data: &mut Value, launch: &Launch, drop_legacy: bool) {
    if !data.is_object() {
        *data = json!({});
    }
    let obj = data.as_object_mut().unwrap();
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    if !servers.is_object() {
        *servers = json!({});
    }
    let map: &mut Map<String, Value> = servers.as_object_mut().unwrap();
    if drop_legacy {
        for name in LEGACY_MCP_NAMES {
            map.remove(name);
        }
    }
    let mut entry = launch.to_json();
    // Claude Code 는 type 을 함께 적는다. 원래 있던 type 은 남긴다
    if let Some(t) = map.get(MCP_NAME).and_then(|e| e.get("type")).cloned() {
        entry["type"] = t;
    }
    map.insert(MCP_NAME.into(), entry);
}

fn is_current(entry: &Value, launch: &Launch) -> bool {
    entry.get("command").and_then(Value::as_str) == Some(launch.command.as_str())
        && entry.get("args") == Some(&json!(launch.args))
        && entry.get("env").and_then(Value::as_object).map(|m| m.is_empty()).unwrap_or(true)
}

/// 이미 smash 가 등록된 곳 중 예전 명령(Electron 실행 파일·옮긴 앱 위치)으로 남은 곳을 지금 앱으로 바꾼다.
/// 등록 안 된 곳은 건드리지 않는다. 바꾼 곳의 이름 목록을 돌려준다.
pub fn refresh_registrations(launch: &Launch) -> Vec<String> {
    let mut changed = Vec::new();
    let mut fix = |file: &Path, label: String| {
        // 바로 전에 다시 읽어서, 그사이 Claude 가 쓴 내용을 덮지 않게 한다
        let Some(mut data) = read_json(file) else { return };
        let Some(entry) = data.get("mcpServers").and_then(|s| s.get(MCP_NAME)) else { return };
        if is_current(entry, launch) {
            return;
        }
        set_server(&mut data, launch, false);
        if write_json_atomic(file, &data).is_ok() {
            changed.push(label);
        }
    };
    for loc in claude_config_locations() {
        fix(&loc.file, loc.label);
    }
    fix(&claude_desktop_config_path(), "Claude Desktop".into());
    changed
}

pub fn integration_status(launch: &Launch) -> Value {
    let cli = cli_status();
    let locations: Vec<Value> = claude_config_locations()
        .into_iter()
        .map(|l| json!({ "id": l.id, "label": l.label, "registered": registered_entry(&l.file).is_some() }))
        .collect();
    let file = claude_desktop_config_path();
    let desktop_installed = file.parent().map(Path::exists).unwrap_or(false);
    let claude_desktop = registered_entry(&file).is_some();
    json!({
        "cli": cli,
        "claudeCode": locations.iter().any(|l| l["registered"] == json!(true)),
        "locations": locations,
        "claudeDesktop": claude_desktop,
        "desktopInstalled": desktop_installed,
        "desktopConfig": contract_home(&file.to_string_lossy()),
        "launch": launch.to_json(),
        "mcpName": MCP_NAME,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::set_lang;
    use crate::test_util::LOCK;

    #[test]
    fn parse_answers() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_lang("ko");
        let s = json!([{ "file": "a.pdf", "dest": "~/Documents", "reason": "x" }]);
        assert_eq!(parse_suggestions(&json!({ "structured_output": { "suggestions": s } }).to_string()).unwrap(), s.as_array().unwrap().clone());
        let text = format!("여기요\n{}", json!({ "suggestions": s }));
        assert_eq!(parse_suggestions(&json!({ "result": text }).to_string()).unwrap(), s.as_array().unwrap().clone());
        let err = parse_suggestions(&json!({ "is_error": true, "result": "Not logged in" }).to_string()).unwrap_err();
        assert!(err.contains("Not logged in"));
        let f = FileInfo { name: "a.pdf".into(), path: "/x/a.pdf".into(), size: 2048.0, mtime_ms: 0.0 };
        assert!(build_prompt("/x", &[f], &["/x".into()]).contains("a.pdf | 2KB | 1970-01-01"));
    }

    #[test]
    fn stale_entries_are_replaced() {
        let launch = Launch { command: "/Applications/Smash.app/Contents/MacOS/smash".into(), args: vec!["--mcp".into()] };
        let mut data = json!({ "other": 1, "mcpServers": { "smash": { "type": "stdio", "command": "/old/Electron", "args": ["server.js"], "env": { "ELECTRON_RUN_AS_NODE": "1" } }, "x": {} } });
        assert!(!is_current(&data["mcpServers"]["smash"], &launch));
        set_server(&mut data, &launch, false);
        assert!(is_current(&data["mcpServers"]["smash"], &launch));
        assert_eq!(data["mcpServers"]["smash"]["type"], json!("stdio"));
        assert_eq!(data["other"], json!(1));
        assert!(data["mcpServers"].get("x").is_some());
    }
}
