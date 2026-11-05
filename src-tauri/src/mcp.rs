//! Smash! MCP 서버 (stdio, 줄 단위 JSON-RPC). `smash --mcp` 로 뜬다.
//! 앱과 같은 설정·기록 파일을 쓰므로 Claude 에서 정리한 내역도 앱의 '기록'에 남는다.

use std::collections::HashMap;
use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::config::{contract_home, load_config, new_id, normalize_match, save_config, Config, Rule};
use crate::lang::{l, resolve_lang, set_lang};
use crate::organizer::{apply_plan, find_target, folder_tree, load_history, plan_from_moves, scan_target, undo_last, Item, MoveReq, Rejected};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const INSTRUCTIONS: &str = "Smash!: 사용자의 폴더(다운로드·문서 등)를 규칙대로 정리하는 도구. scan_folder로 계획을 만들고, 사용자에게 보여 준 뒤 apply_plan으로 실행한다. 삭제 없이 휴지통으로만 보내며 undo_last로 되돌릴 수 있다.";

fn tools() -> Value {
    json!([
        {
            "name": "list_targets",
            "description": "정리 대상으로 등록된 폴더와 각 폴더의 규칙을 보여 준다.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "scan_folder",
            "description": "정리 대상 폴더를 스캔해 규칙에 따른 정리 계획(plan)과 규칙에 안 걸린 파일(unmatched), 옮길 수 있는 폴더 목록(folders)을 돌려준다. 파일은 아직 움직이지 않는다. unmatched 파일은 이름을 보고 folders 중 알맞은 곳을 골라 apply_plan의 moves로 넘기면 된다.",
            "inputSchema": { "type": "object", "properties": { "target": { "type": "string", "description": "폴더 id(예: downloads) 또는 경로(예: ~/Downloads). 생략하면 첫 번째 폴더." } } }
        },
        {
            "name": "apply_plan",
            "description": "정리를 실제로 실행한다. 실행 전에 무엇을 어디로 옮기는지 사용자에게 보여 주고 동의를 받을 것. item_ids는 scan_folder가 준 계획 항목, moves는 직접 정한 이동이다. 삭제는 없고 휴지통으로만 보낸다. 실행 내역은 undo_last로 되돌릴 수 있다.",
            "inputSchema": { "type": "object", "properties": {
                "item_ids": { "type": "array", "items": { "type": "string" }, "description": "scan_folder 결과 plan의 id 목록" },
                "moves": { "type": "array", "items": { "type": "object", "properties": {
                    "src": { "type": "string", "description": "옮길 파일 경로" },
                    "dest_folder": { "type": "string", "description": "목적지 폴더 경로(절대 경로, ~ 경로, 또는 파일이 있는 폴더 기준 상대 경로)" },
                    "action": { "type": "string", "enum": ["move", "trash"] },
                    "reason": { "type": "string" }
                }, "required": ["src"] } }
            } }
        },
        {
            "name": "add_rule",
            "description": "정리 규칙을 추가한다. 확장자·이름 포함 문자열·오래된 날짜 중 하나 이상의 조건이 필요하다. 앞에 있는 규칙이 먼저 적용된다.",
            "inputSchema": { "type": "object", "properties": {
                "target": { "type": "string", "description": "폴더 id 또는 경로" },
                "name": { "type": "string" },
                "extensions": { "type": "array", "items": { "type": "string" }, "description": "예: [\"pdf\",\"hwp\"]" },
                "name_includes": { "type": "array", "items": { "type": "string" }, "description": "파일 이름에 들어 있으면 매칭(대소문자 무시)" },
                "older_than_days": { "type": "number" },
                "action": { "type": "string", "enum": ["move", "trash"] },
                "dest": { "type": "string", "description": "move일 때 목적지. 상대 경로면 대상 폴더 기준." },
                "position": { "type": "string", "enum": ["first", "last"], "description": "기본 first" }
            }, "required": ["target", "name", "action"] }
        },
        {
            "name": "undo_last",
            "description": "가장 최근 정리 실행을 되돌린다(옮긴 파일을 원래 자리로). 휴지통으로 보낸 파일은 사용자가 휴지통에서 직접 복원해야 한다.",
            "inputSchema": { "type": "object", "properties": {} }
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
    /// scan_folder 가 만든 계획을 apply_plan 에서 id 로 받을 수 있게 잠시 들고 있는다
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
                        "rules": t.rules.iter().map(|r| json!({
                            "name": r.name,
                            "enabled": r.enabled,
                            "when": r.matcher,
                            "action": r.action,
                            "dest": if r.dest.is_empty() { Value::Null } else { json!(r.dest) },
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
                    "folders": folder_tree(&cfg, 2).iter().map(|f| contract_home(f)).collect::<Vec<_>>(),
                }))
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
                            reason: l("계획에 없는 id입니다. scan_folder를 다시 실행하세요.", "Unknown id. Run scan_folder again."),
                        }),
                    }
                }
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
                items.extend(planned.items);
                rejected.extend(planned.rejected);
                let applied = apply_plan(&cfg, items, "claude-mcp");
                for id in &ids {
                    self.last_plan.remove(id);
                }
                Ok(json!({ "runId": applied.run.as_ref().map(|r| r.id.clone()), "results": applied.results, "rejected": rejected }))
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
                })));
                if matcher.extensions.is_empty() && matcher.name_includes.is_empty() && matcher.older_than_days <= 0.0 {
                    return Err(l("조건이 하나 이상 필요합니다", "At least one condition is needed"));
                }
                let name = args.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let rule = Rule {
                    id: new_id("r"),
                    name: name.clone(),
                    enabled: true,
                    matcher,
                    action: action.into(),
                    dest: if action == "trash" { String::new() } else { dest },
                };
                let last = args.get("position").and_then(Value::as_str) == Some("last");
                let path = crate::organizer::add_rule(&mut cfg, &target, rule, last)?;
                save_config(&cfg)?;
                Ok(json!({ "added": name, "target": path }))
            }
            "undo_last" => Ok(undo_last()),
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
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 6);
        let scan = s.handle(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "scan_folder", "arguments": { "target": "inbox" } } })).unwrap();
        assert!(scan["result"].get("isError").is_none(), "{scan}");
        let bad = s.handle(&json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "apply_plan", "arguments": { "item_ids": ["nope"] } } })).unwrap();
        assert!(bad["result"]["content"][0]["text"].as_str().unwrap().contains("계획에 없는 id"));
        let rule = s.handle(&json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "add_rule", "arguments": { "target": "inbox", "name": "x", "action": "move" } } })).unwrap();
        assert_eq!(rule["result"]["isError"], json!(true));
        let unknown = s.handle(&json!({ "jsonrpc": "2.0", "id": 6, "method": "nope" })).unwrap();
        assert_eq!(unknown["error"]["code"], json!(-32601));
    }
}
