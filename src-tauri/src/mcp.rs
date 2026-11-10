//! Smash! MCP 서버 (stdio, 줄 단위 JSON-RPC). `smash --mcp` 로 뜬다.
//! 앱과 같은 설정·기록 파일을 쓰므로 Claude 에서 정리한 내역도 앱의 '기록'에 남는다.

use std::collections::HashMap;
use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::config::{contract_home, load_config, new_id, normalize_match, save_config, Config, Rule};
use crate::lang::{l, resolve_lang, set_lang};
use crate::organizer::{apply_plan, find_target, folder_tree, load_history, plan_from_moves, scan_target, undo_run, Item, MoveReq, Rejected};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const INSTRUCTIONS: &str = "Smash!: 사용자의 폴더(다운로드·문서 등)를 규칙대로 정리하는 도구. scan_folder(규칙대로)나 plan_moves(직접 정한 이동)로 계획을 만들고, 사용자에게 보여 준 뒤 그 계획의 id로 apply_plan을 실행한다. 계획에 없는 이동은 실행되지 않는다. 삭제 없이 휴지통으로만 보내며 undo_last로 되돌릴 수 있다.";

fn tools() -> Value {
    json!([
        {
            "name": "list_targets",
            "description": "정리 대상으로 등록된 폴더와 각 폴더의 규칙을 보여 준다.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "scan_folder",
            "description": "정리 대상 폴더를 스캔해 규칙에 따른 정리 계획(plan)과 규칙에 안 걸린 파일(unmatched), 옮길 수 있는 폴더 목록(folders)을 돌려준다. 파일은 아직 움직이지 않는다. unmatched 파일은 이름을 보고 folders 중 알맞은 곳을 골라 plan_moves로 계획에 더하면 된다. blocked는 규칙의 목적지가 정리 대상 폴더 밖이라 실행할 수 없는 항목이다.",
            "inputSchema": { "type": "object", "properties": { "target": { "type": "string", "description": "폴더 id(예: downloads) 또는 경로(예: ~/Downloads). 생략하면 첫 번째 폴더." } } }
        },
        {
            "name": "plan_moves",
            "description": "직접 정한 이동을 검증해 계획 항목(id 포함)으로 만든다. 파일은 아직 움직이지 않는다. 정리 대상 폴더 밖의 파일·목적지는 rejected로 돌려준다. 돌려받은 plan을 사용자에게 보여 주고 동의를 받은 뒤 그 id를 apply_plan에 넘긴다.",
            "inputSchema": { "type": "object", "properties": {
                "moves": { "type": "array", "items": { "type": "object", "properties": {
                    "src": { "type": "string", "description": "옮길 파일 경로" },
                    "dest_folder": { "type": "string", "description": "목적지 폴더 경로(절대 경로, ~ 경로, 또는 파일이 있는 폴더 기준 상대 경로)" },
                    "action": { "type": "string", "enum": ["move", "trash"] },
                    "reason": { "type": "string" }
                }, "required": ["src"] } }
            }, "required": ["moves"] }
        },
        {
            "name": "apply_plan",
            "description": "정리를 실제로 실행한다. 실행 전에 무엇을 어디로 옮기는지 사용자에게 보여 주고 동의를 받을 것. scan_folder나 plan_moves가 돌려준 계획 항목의 id만 받는다(계획에 없는 이동은 실행하지 않는다). 삭제는 없고 휴지통으로만 보낸다. 실행 내역은 undo_last로 되돌릴 수 있다.",
            "inputSchema": { "type": "object", "properties": {
                "item_ids": { "type": "array", "items": { "type": "string" }, "description": "scan_folder·plan_moves 결과 plan의 id 목록" }
            }, "required": ["item_ids"] }
        },
        {
            "name": "add_rule",
            "description": "정리 규칙을 추가한다. 확장자·이름 포함 문자열·받은 지 N일·받은 곳 중 하나 이상의 조건이 필요하다. 앞에 있는 규칙이 먼저 적용된다. move 규칙의 dest는 정리 대상 폴더 안이어야 실행된다. 자동 정리가 켜진 폴더에는 꺼진 상태로 들어가며, 사용자가 앱에서 켜야 적용된다.",
            "inputSchema": { "type": "object", "properties": {
                "target": { "type": "string", "description": "폴더 id 또는 경로" },
                "name": { "type": "string" },
                "extensions": { "type": "array", "items": { "type": "string" }, "description": "예: [\"pdf\",\"hwp\"]" },
                "name_includes": { "type": "array", "items": { "type": "string" }, "description": "파일 이름에 들어 있으면 매칭(대소문자 무시)" },
                "older_than_days": { "type": "number", "description": "받은 지 N일 지난 파일(폴더에 들어온 날과 마지막으로 고친 날 중 늦은 쪽부터 센다)" },
                "from_includes": { "type": "array", "items": { "type": "string" }, "description": "내려받은 주소에 들어 있으면 매칭(예: [\"github.com\"]). macOS·Windows에서 받은 곳 기록이 있는 파일만." },
                "action": { "type": "string", "enum": ["move", "trash"] },
                "dest": { "type": "string", "description": "move일 때 목적지. 상대 경로면 대상 폴더 기준." },
                "group": { "type": "string", "enum": ["", "auto", "month"], "description": "목적지 안 소분류. auto=용도(영수증·스크린샷 등)·비슷한 이름·찍은 시간으로 하위 폴더, month=연-월, 빈 문자열=없음. 기본 auto" },
                "position": { "type": "string", "enum": ["first", "last"], "description": "기본 first" }
            }, "required": ["target", "name", "action"] }
        },
        {
            "name": "undo_last",
            "description": "정리 실행 하나를 되돌린다(옮긴 파일을 원래 자리로). run_id를 주면 그 실행을, 없으면 아직 돌릴 것이 남은 가장 최근 실행을 되돌린다. 휴지통으로 보낸 파일은 휴지통 안 위치를 아는 경우(macOS·Linux)에만 꺼내 오고, 못 꺼낸 것은 trashed 목록으로 알려 준다. 일부가 실패하면 그 파일만 남아 다시 시도할 수 있다.",
            "inputSchema": { "type": "object", "properties": { "run_id": { "type": "string", "description": "history가 돌려준 실행 id" } } }
        },
        {
            "name": "history",
            "description": "최근 정리 실행 기록을 보여 준다.",
            "inputSchema": { "type": "object", "properties": { "limit": { "type": "number" } } }
        }
    ])
}

fn view(item: &Item) -> Value {
    let note = match item.note.as_deref() {
        Some("outside") => Some(l("규칙의 목적지가 정리 대상 폴더 밖이라 실행할 수 없음", "The rule's destination is outside the folders you set up, so it can't run")),
        Some("duplicate") => Some(format!(
            "{}{}",
            l("같은 내용 파일이 이미 있음: ", "A file with the same content already exists: "),
            contract_home(item.duplicate_of.as_deref().unwrap_or(""))
        )),
        Some("renamed") => Some(l("같은 이름이 있어 이름 변경", "Renamed because the name was taken")),
        _ => None,
    };
    json!({
        "id": item.id,
        "file": contract_home(&item.src),
        "action": item.action,
        "dest": if item.dest.is_empty() { Value::Null } else { json!(contract_home(&item.dest)) },
        "note": note,
        "reason": item.reason,
    })
}

fn strs(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default()
}

pub struct Server {
    /// scan_folder·plan_moves 가 만든 계획. apply_plan 은 여기 있는 id 만 실행한다.
    last_plan: HashMap<String, Item>,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    pub fn new() -> Self {
        Server { last_plan: HashMap::new() }
    }

    fn cfg() -> Result<Config, String> {
        load_config()
    }

    fn call(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "list_targets" => {
                let cfg = Self::cfg()?;
                Ok(json!(cfg
                    .targets
                    .iter()
                    .map(|t| json!({
                        "id": t.id,
                        "path": t.path,
                        "includeSubfolders": t.recursive,
                        "sharesRulesOf": if t.link.is_empty() { Value::Null } else { json!(t.link) },
                        "rules": t.rules.iter().map(|r| json!({
                            "name": r.name,
                            "enabled": r.enabled,
                            "when": r.matcher,
                            "action": r.action,
                            "dest": if r.dest.is_empty() { Value::Null } else { json!(r.dest) },
                            "group": r.group,
                        })).collect::<Vec<_>>(),
                    }))
                    .collect::<Vec<_>>()))
            }
            "scan_folder" => {
                let cfg = Self::cfg()?;
                let res = scan_target(&cfg, args.get("target").and_then(Value::as_str))?;
                self.last_plan.clear();
                for item in &res.items {
                    self.last_plan.insert(item.id.clone(), item.clone());
                }
                Ok(json!({
                    "target": contract_home(&res.root),
                    "plan": res.items.iter().map(view).collect::<Vec<_>>(),
                    "unmatched": res.unmatched.iter().map(|f| json!({
                        "file": contract_home(&f.path),
                        "sizeKB": (f.size / 1024.0).round() as i64,
                        "modified": chrono::DateTime::from_timestamp_millis(f.mtime_ms as i64).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default(),
                    })).collect::<Vec<_>>(),
                    "blocked": res.blocked.iter().map(view).collect::<Vec<_>>(),
                    "truncated": res.truncated,
                    "folders": folder_tree(&cfg, 2).iter().map(|f| contract_home(f)).collect::<Vec<_>>(),
                }))
            }
            "plan_moves" => {
                let cfg = Self::cfg()?;
                let moves: Vec<MoveReq> = args
                    .get("moves")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|m| {
                                let src = m.get("src")?.as_str()?.to_string();
                                let reason = m.get("reason").and_then(Value::as_str).filter(|s| !s.is_empty());
                                Some(MoveReq {
                                    src,
                                    dest_folder: m.get("dest_folder").and_then(Value::as_str).map(String::from),
                                    action: m.get("action").and_then(Value::as_str).unwrap_or("move").to_string(),
                                    reason: reason.map(|r| format!("Claude: {r}")).unwrap_or_else(|| "Claude".into()),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let planned = plan_from_moves(&cfg, moves);
                for item in &planned.items {
                    self.last_plan.insert(item.id.clone(), item.clone());
                }
                Ok(json!({ "plan": planned.items.iter().map(view).collect::<Vec<_>>(), "rejected": planned.rejected }))
            }
            "apply_plan" => {
                let cfg = Self::cfg()?;
                let ids = strs(args.get("item_ids"));
                let mut items = Vec::new();
                let mut rejected = Vec::new();
                for id in &ids {
                    match self.last_plan.get(id) {
                        Some(item) => items.push(item.clone()),
                        None => rejected.push(Rejected {
                            src: id.clone(),
                            reason: l("계획에 없는 id입니다. scan_folder나 plan_moves를 다시 실행하세요.", "Unknown id. Run scan_folder or plan_moves again."),
                        }),
                    }
                }
                // 보여 준 적 없는 이동은 실행하지 않는다
                if args.get("moves").is_some_and(|m| m.as_array().is_some_and(|a| !a.is_empty())) {
                    return Err(l(
                        "apply_plan은 계획 id만 받습니다. 직접 정한 이동은 plan_moves로 먼저 계획을 만들고, 사용자에게 보여 준 뒤 그 id를 넘기세요.",
                        "apply_plan only takes plan ids. Make a plan with plan_moves first, show it to the user, then pass its ids.",
                    ));
                }
                let applied = apply_plan(&cfg, items, "claude-mcp");
                for id in &ids {
                    self.last_plan.remove(id);
                }
                Ok(json!({ "runId": applied.run.as_ref().map(|r| r.id.clone()), "results": applied.results, "rejected": rejected, "historyError": applied.history_error }))
            }
            "add_rule" => {
                let mut cfg = Self::cfg()?;
                let target = args.get("target").and_then(Value::as_str).unwrap_or("").to_string();
                if find_target(&cfg, Some(&target)).is_none() {
                    return Err(format!("{}{target}", l("설정에 없는 폴더입니다: ", "Folder not in settings: ")));
                }
                let action = args.get("action").and_then(Value::as_str).unwrap_or("");
                if action != "move" && action != "trash" {
                    return Err(l("action은 move 또는 trash여야 합니다", "action must be move or trash"));
                }
                let dest = args.get("dest").and_then(Value::as_str).unwrap_or("").to_string();
                if action == "move" && dest.is_empty() {
                    return Err(l("move 규칙에는 dest가 필요합니다", "A move rule needs dest"));
                }
                let matcher = normalize_match(Some(&json!({
                    "extensions": args.get("extensions").cloned().unwrap_or(json!([])),
                    "nameIncludes": args.get("name_includes").cloned().unwrap_or(json!([])),
                    "olderThanDays": args.get("older_than_days").cloned().unwrap_or(json!(0)),
                    "fromIncludes": args.get("from_includes").cloned().unwrap_or(json!([])),
                })));
                if matcher.extensions.is_empty() && matcher.name_includes.is_empty() && matcher.from_includes.is_empty() && matcher.older_than_days <= 0.0 {
                    return Err(l("조건이 하나 이상 필요합니다", "At least one condition is needed"));
                }
                let name = args.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                // 자동 정리가 켜진 폴더는 미리보기 없이 실행된다. Claude 가 넣은 규칙은 꺼 둔 채로 넣고 사용자가 앱에서 켜게 한다.
                let auto_on = find_target(&cfg, Some(&target)).is_some_and(|t| t.auto);
                let rule = Rule {
                    id: new_id("r"),
                    name: name.clone(),
                    enabled: !auto_on,
                    matcher,
                    action: action.into(),
                    dest: if action == "trash" { String::new() } else { dest },
                    group: match args.get("group").and_then(Value::as_str) {
                        _ if action == "trash" => String::new(),
                        Some(g) if crate::config::GROUP_MODES.contains(&g) => g.into(),
                        _ => "auto".into(),
                    },
                };
                // 정리 대상 밖을 가리키면 저장은 하되 실행되지 않는다는 것을 알려 준다
                let outside = find_target(&cfg, Some(&target)).is_some_and(|t| !crate::organizer::rule_dest_inside(&cfg, t, &rule));
                let last = args.get("position").and_then(Value::as_str) == Some("last");
                let path = crate::organizer::add_rule(&mut cfg, &target, rule, last)?;
                save_config(&cfg)?;
                let mut out = json!({ "added": name, "target": path, "enabled": !auto_on });
                if auto_on {
                    out["note"] = json!(l(
                        "이 폴더는 자동 정리가 켜져 있어 규칙을 꺼진 상태로 넣었습니다. 사용자가 앱의 규칙 탭에서 켜야 적용됩니다.",
                        "Auto-tidy is on for this folder, so the rule was added switched off. The user has to turn it on in the app's Rules tab.",
                    ));
                }
                if outside {
                    out["warning"] = json!(l(
                        "목적지가 정리 대상 폴더 밖이라 이 규칙은 실행되지 않습니다. 그 폴더를 앱에서 정리할 폴더로 추가하세요.",
                        "The destination is outside the folders you set up, so this rule won't run. Add that folder in the app.",
                    ));
                }
                Ok(out)
            }
            "undo_last" => Ok(undo_run(args.get("run_id").and_then(Value::as_str))),
            "history" => {
                let limit = args.get("limit").and_then(Value::as_f64).unwrap_or(5.0).max(0.0) as usize;
                let runs = load_history();
                let start = runs.len().saturating_sub(limit);
                Ok(json!(runs[start..]
                    .iter()
                    .rev()
                    .map(|r| json!({
                        "id": r.id,
                        "at": r.at,
                        "source": r.source,
                        "undone": r.undone,
                        "state": r.state,
                        "leftToUndo": r.retryable(),
                        "failed": r.failed.iter().map(|f| format!("{}: {}", contract_home(&f.file), f.reason)).collect::<Vec<_>>(),
                        "ops": r.ops.iter().map(|o| if o.kind == "move" {
                            format!("{} → {}", contract_home(&o.from), contract_home(o.to.as_deref().unwrap_or("")))
                        } else {
                            format!("{}{}", l("휴지통: ", "Trash: "), contract_home(&o.from))
                        }).collect::<Vec<_>>(),
                    }))
                    .collect::<Vec<_>>()))
            }
            _ => Err(String::new()),
        }
    }

    /// 메시지 하나를 처리하고, 답할 게 있으면 돌려준다
    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let empty = json!({});
        let params = msg.get("params").unwrap_or(&empty);
        let reply = |result: Value| id.clone().map(|id| json!({ "jsonrpc": "2.0", "id": id, "result": result }));
        let fail = |code: i64, message: String| id.clone().map(|id| json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }));
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
                let version = if PROTOCOL_VERSIONS.contains(&asked) { asked } else { PROTOCOL_VERSIONS[0] };
                reply(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "smash", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => reply(json!({})),
            "tools/list" => reply(json!({ "tools": tools() })),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                if !tools().as_array().unwrap().iter().any(|t| t["name"] == name) {
                    return fail(-32602, format!("{}{name}", l("없는 도구: ", "Unknown tool: ")));
                }
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match self.call(name, &args) {
                    Ok(out) => reply(json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&out).unwrap_or_default() }] })),
                    Err(e) => reply(json!({ "content": [{ "type": "text", "text": format!("{}{e}", l("오류: ", "Error: ")) }], "isError": true })),
                }
            }
            m if m.starts_with("notifications/") => None,
            m => fail(-32601, format!("{}{m}", l("지원하지 않는 메서드: ", "Unsupported method: "))),
        }
    }
}

pub fn run() {
    // 앱에서 고른 언어로 오류·결과 문구를 돌려준다
    let setting = load_config().ok().map(|c| c.settings.language);
    set_lang(resolve_lang(setting.as_deref(), None));
    let mut server = Server::new();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => server.handle(&msg),
            Err(_) => Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Parse error" } })),
        };
        if let Some(a) = answer {
            let _ = writeln!(out, "{a}");
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{sandbox, write, LOCK};

    #[test]
    fn json_rpc_contract() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_lang("ko");
        let sb = sandbox();
        let _ = std::fs::remove_dir_all(sb.join("config"));
        let inbox = sb.join("mcp-inbox");
        let _ = std::fs::remove_dir_all(&inbox);
        write(&inbox.join("memo.pdf"), "x", 0);
        let mut cfg = crate::config::default_config();
        cfg.targets.truncate(1);
        cfg.targets[0].path = inbox.to_string_lossy().into_owned();
        cfg.targets[0].id = "inbox".into();
        save_config(&cfg).unwrap();

        let mut s = Server::new();
        let init = s.handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2024-11-05" } })).unwrap();
        assert_eq!(init["result"]["protocolVersion"], json!("2024-11-05"));
        assert!(s.handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
        let list = s.handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 7);
        let scan = s.handle(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "scan_folder", "arguments": { "target": "inbox" } } })).unwrap();
        assert!(scan["result"].get("isError").is_none(), "{scan}");
        let bad = s.handle(&json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "apply_plan", "arguments": { "item_ids": ["nope"] } } })).unwrap();
        assert!(bad["result"]["content"][0]["text"].as_str().unwrap().contains("계획에 없는 id"));
        let rule = s.handle(&json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "add_rule", "arguments": { "target": "inbox", "name": "x", "action": "move" } } })).unwrap();
        assert_eq!(rule["result"]["isError"], json!(true));
        let unknown = s.handle(&json!({ "jsonrpc": "2.0", "id": 6, "method": "nope" })).unwrap();
        assert_eq!(unknown["error"]["code"], json!(-32601));

        // 보여 준 계획만 실행한다: 자유 이동은 거절, plan_moves 가 준 id 는 실행
        let call = |s: &mut Server, id: u32, name: &str, args: Value| -> Value {
            let res = s.handle(&json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": args } })).unwrap();
            let text = res["result"]["content"][0]["text"].as_str().unwrap().to_string();
            if res["result"]["isError"] == json!(true) { json!({ "error": text }) } else { serde_json::from_str(&text).unwrap() }
        };
        let src = inbox.join("memo.pdf").to_string_lossy().into_owned();
        let direct = call(&mut s, 7, "apply_plan", json!({ "moves": [{ "src": src, "dest_folder": "메모" }] }));
        assert!(direct["error"].as_str().unwrap().contains("plan_moves"), "{direct}");
        assert!(inbox.join("memo.pdf").exists(), "계획 없이 넘긴 이동은 실행되지 않는다");
        let planned = call(&mut s, 8, "plan_moves", json!({ "moves": [
            { "src": src, "dest_folder": "메모", "reason": "메모 파일" },
            { "src": src, "dest_folder": sb.join("elsewhere") }
        ] }));
        assert_eq!(planned["rejected"].as_array().unwrap().len(), 1, "밖으로 가는 이동은 계획 단계에서 거절");
        let id = planned["plan"][0]["id"].as_str().unwrap().to_string();
        assert!(inbox.join("memo.pdf").exists(), "plan_moves 는 파일을 옮기지 않는다");
        let done = call(&mut s, 9, "apply_plan", json!({ "item_ids": [id] }));
        assert_eq!(done["results"][0]["ok"], json!(true), "{done}");
        assert!(inbox.join("메모").join("memo.pdf").exists());
        let again = call(&mut s, 10, "apply_plan", json!({ "item_ids": [id] }));
        assert_eq!(again["rejected"].as_array().unwrap().len(), 1, "한 번 쓴 id 는 다시 못 쓴다");
        // 실행 id 로 골라 되돌리기
        let run_id = done["runId"].as_str().unwrap().to_string();
        let undone = call(&mut s, 11, "undo_last", json!({ "run_id": run_id }));
        assert_eq!(undone["restored"], json!(1));
        assert!(inbox.join("memo.pdf").exists());
        // 밖을 가리키는 규칙은 경고와 함께 저장된다
        // 자동 정리가 켜진 폴더에 Claude 가 넣은 규칙은 꺼진 채로 들어간다(사용자가 앱에서 켜야 실행된다)
        let mut auto_cfg = load_config().unwrap();
        auto_cfg.targets[0].auto = true;
        save_config(&auto_cfg).unwrap();
        let held = call(&mut s, 13, "add_rule", json!({ "target": "inbox", "name": "전부 버리기", "action": "trash", "extensions": ["pdf"] }));
        assert_eq!(held["enabled"], json!(false), "{held}");
        assert!(!load_config().unwrap().targets[0].rules.iter().find(|r| r.name == "전부 버리기").unwrap().enabled);
        auto_cfg.targets[0].auto = false;
        save_config(&auto_cfg).unwrap();
        let warned = call(&mut s, 12, "add_rule", json!({ "target": "inbox", "name": "밖", "action": "move", "extensions": ["zip"], "dest": sb.join("elsewhere") }));
        assert!(warned["warning"].is_string(), "{warned}");
    }
}
