//! Smash! 앱 본체: 창, 트레이, 스매시 효과, 잔소리 배너, 화면에서 부르는 명령, 자동 업데이트.
//! 정리 로직은 organizer·watcher, Claude 연동은 claude, MCP 서버는 mcp 에 있다.

pub mod claude;
pub mod config;
pub mod fsops;
pub mod grouping;
pub mod history;
pub mod lang;
pub mod mcp;
pub mod organizer;
pub mod thumbs;
pub mod trash;
pub mod watcher;

#[cfg(test)]
mod test_util;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::menu::{CheckMenuItem, MenuBuilder, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as AutostartExt};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Update, UpdaterExt};

use claude::Launch;
use config::{contract_home, expand_home, load_config, save_config, save_config_value, Config};
use lang::{get_lang, l, resolve_lang, set_lang};
use organizer::{apply_all, apply_into, find_target, missing_dirs, scan_target_with, Applied, Item, Op, Recorder};
use watcher::AutoTidy;

const APP_NAME: &str = "Smash!";
/// 배너는 화면 쪽(nag.js)이 스스로 닫는다. 닫지 못했을 때를 위한 상한
const NAG_MAX_MS: u64 = 60_000;
/// 이 수 이상 쌓이면 잔소리 단계가 오른다
const NAG_STEPS: [usize; 3] = [20, 60, 150];
const UPDATE_FIRST_CHECK: Duration = Duration::from_secs(15);
const UPDATE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

// ── 상태 ──────────────────────────────────────────────

struct Auto(Mutex<Option<AutoTidy>>);
/// 정리 버튼과 트레이의 정리가 겹치면 앞의 효과를 끊지 않고 차례로 돌린다
struct SmashQueue(tokio::sync::Mutex<()>);
/// 잔소리 배너를 누르면 할 일
#[derive(Clone)]
enum NagAction {
    /// 정리한 폴더를 연다
    Folder(String),
    /// 앱 창에서 그 폴더의 미리보기를 보여 준다
    Preview(String),
    /// 기록 탭을 연다
    History,
}
struct NagFolder(Mutex<Option<NagAction>>);
/// 앱이 마지막으로 쓴(또는 읽은) 설정 파일의 상태. 다른 프로그램이 바꿨는지 가릴 때 쓴다.
#[derive(Default)]
struct ConfigRev(Mutex<String>);
struct Quitting(AtomicBool);
#[derive(Default)]
struct PendingUpdate(Mutex<Option<(Update, Vec<u8>)>>);
/// 설정 화면과 트레이가 같이 보는 업데이트 상태
#[derive(Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInfo {
    /// 확인하거나 받는 중
    checking: bool,
    /// 이번 실행에서 한 번이라도 확인을 마쳤는지
    checked: bool,
    /// 마지막 확인이나 설치가 실패했는지
    error: bool,
    /// 찾은 새 버전 (받는 중이거나 받아 둠)
    latest: Option<String>,
    /// 받아 둬서 바로 설치할 수 있는지
    ready: bool,
}
#[derive(Default)]
struct UpdateStatus(Mutex<UpdateInfo>);

fn cfg_or_default() -> Config {
    load_config().unwrap_or_else(|_| config::default_config())
}

fn any_auto(cfg: &Config) -> bool {
    cfg.targets.iter().any(|t| t.auto)
}

fn label(p: &str) -> String {
    if p.is_empty() {
        String::new()
    } else {
        contract_home(p)
    }
}

fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "win32"
    } else {
        "linux"
    }
}

/// 화면에 보여 줄 짧은 경로와 새로 만들 폴더를 붙인다
fn decorate(items: &[Item]) -> Value {
    // 같은 폴더로 가는 파일이 많다. 폴더가 있는지는 폴더마다 한 번만 본다.
    let mut seen: std::collections::HashMap<PathBuf, Vec<String>> = std::collections::HashMap::new();
    json!(items
        .iter()
        .map(|it| {
            let mut v = json!(it);
            v["srcLabel"] = json!(label(&it.src));
            v["destLabel"] = json!(label(&it.dest));
            v["duplicateLabel"] = json!(label(it.duplicate_of.as_deref().unwrap_or("")));
            let dirs: Vec<String> = if it.action == "move" && !it.dest.is_empty() {
                let dest = Path::new(&it.dest);
                seen.entry(dest.parent().map(Path::to_path_buf).unwrap_or_default())
                    .or_insert_with(|| missing_dirs(dest).iter().map(|d| d.to_string_lossy().into_owned()).collect())
                    .clone()
            } else {
                vec![]
            };
            v["newDirs"] = json!(dirs);
            v
        })
        .collect::<Vec<_>>())
}

fn to_value<T: serde::Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// 무거운 일(파일 이동·Claude CLI)은 화면을 막지 않게 따로 돌린다
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

// ── 설정 적용 ─────────────────────────────────────────

fn sync_lang(cfg: &Config) {
    set_lang(resolve_lang(Some(&cfg.settings.language), None));
}

fn apply_settings(app: &AppHandle, cfg: &Config) {
    sync_lang(cfg);
    if let Some(a) = app.state::<Auto>().0.lock().unwrap().as_ref() {
        a.sync(Some(cfg.clone()));
    }
    // 개발 빌드는 로그인 항목에 넣지 않는다. 값이 바뀔 때만 건드린다
    if !cfg!(debug_assertions) && !cfg!(target_os = "linux") {
        let al = app.autolaunch();
        if al.is_enabled().unwrap_or(false) != cfg.settings.open_at_login {
            let res = if cfg.settings.open_at_login { al.enable() } else { al.disable() };
            if let Err(e) = res {
                eprintln!("로그인 항목을 바꾸지 못했습니다: {e}");
            }
        }
    }
    update_tray(app);
}

// ── 창 ────────────────────────────────────────────────

fn set_dock(app: &AppHandle, visible: bool) {
    #[cfg(target_os = "macos")]
    {
        let policy = if visible { tauri::ActivationPolicy::Regular } else { tauri::ActivationPolicy::Accessory };
        let _ = app.set_activation_policy(policy);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, visible);
}

fn is_app_url(url: &tauri::Url) -> bool {
    matches!(url.scheme(), "tauri" | "asset") || url.host_str() == Some("tauri.localhost") || url.host_str() == Some("localhost")
}

fn create_main(app: &AppHandle, show: bool) -> tauri::Result<WebviewWindow> {
    let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("renderer/index.html".into()))
        .title(APP_NAME)
        .inner_size(1180.0, 800.0)
        .min_inner_size(860.0, 580.0)
        .visible(show)
        // 화면 밖으로 나가는 이동은 막는다. 외부 링크는 openUrl 로 브라우저에서 연다
        .on_navigation(is_app_url)
        .build()?;
    let handle = app.clone();
    win.on_window_event(move |e| {
        // 창을 닫아도 앱은 트레이(메뉴 막대·알림 영역)에 남는다. 끝내기는 트레이 메뉴의 '종료'.
        if let WindowEvent::CloseRequested { api, .. } = e {
            // 리눅스는 트레이가 없을 수 있어 예전처럼 자동 정리를 켰을 때만 숨긴다
            let keep = !cfg!(target_os = "linux") || any_auto(&cfg_or_default());
            if keep && !handle.state::<Quitting>().0.load(Ordering::SeqCst) {
                api.prevent_close();
                if let Some(w) = handle.get_webview_window("main") {
                    let _ = w.hide();
                }
                set_dock(&handle, false);
                tell_still_running(&handle);
            }
        }
    });
    Ok(win)
}

/// 윈도우는 X 를 누르면 끝난 줄 알기 쉬워서, 처음 숨길 때 한 번 알림 영역에 있다고 알린다.
fn tell_still_running(app: &AppHandle) {
    static TOLD: AtomicBool = AtomicBool::new(false);
    if !cfg!(target_os = "windows") || TOLD.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = app
        .notification()
        .builder()
        .title(l(&format!("{APP_NAME} 는 알림 영역에서 계속 실행 중이에요"), &format!("{APP_NAME} is still running in the tray")))
        .body(l("끝내려면 알림 영역 아이콘 메뉴에서 '종료'를 누르세요.", "To quit, use 'Quit' in the tray icon menu."))
        .show();
}

fn show_window(app: &AppHandle, tab: Option<&str>) {
    set_dock(app, true);
    let win = match app.get_webview_window("main") {
        Some(w) => w,
        None => match create_main(app, true) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("창을 만들지 못했습니다: {e}");
                return;
            }
        },
    };
    let _ = win.unminimize();
    let _ = win.show();
    let _ = win.set_focus();
    if let Some(tab) = tab {
        let _ = win.emit("ui:tab", tab);
    }
}

#[cfg(target_os = "macos")]
fn join_all_spaces(w: &WebviewWindow) {
    let w2 = w.clone();
    let _ = w.run_on_main_thread(move || {
        use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
        if let Ok(ptr) = w2.ns_window() {
            // SAFETY: ns_window 는 이 창의 NSWindow 포인터이고, 메인 스레드에서 부른다
            let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
            #[allow(unused_unsafe)]
            unsafe {
                ns.setCollectionBehavior(
                    ns.collectionBehavior() | NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::FullScreenAuxiliary,
                );
            }
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn join_all_spaces(w: &WebviewWindow) {
    let _ = w.set_visible_on_all_workspaces(true);
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// 투명·테두리 없는 맨 위 창 (스매시 효과, 잔소리 배너)
fn overlay<'a>(app: &'a AppHandle, label: &str, url: String, on_load: impl Fn() + Send + Sync + 'static) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .focused(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .visible(false)
        // 포커스를 안 가져가는 창이라, 첫 클릭이 창 활성화에 먹히지 않게 한다 (잔소리 배너 클릭)
        .accept_first_mouse(true)
        .on_navigation(is_app_url)
        .on_page_load(move |_w, p| {
            if p.event() == PageLoadEvent::Finished {
                on_load();
            }
        })
}

// ── 등짝 스매시 효과 ──────────────────────────────────
// 화면 맨 위에 투명 창을 띄워 손바닥이 크게 나타났다가 "찰싹!!" 내리치게 하고, 앱 창도 같이 들썩인다.

/// 파일 수에 따른 세기. smash.css 의 타임라인과 맞춘다.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Blow {
    /// smash.html 에 넘기는 이름: tap(톡) · slap(찰싹!!) · wham(철썩!!!)
    level: &'static str,
    /// 내리치는 순간(ms)과 효과 전체 길이(ms)
    hit_ms: u64,
    total_ms: u64,
    /// 앱 창 흔들림 배율 (0 이면 안 흔든다)
    shake: f64,
}

/// 1~5개는 손가락으로 톡, 6~50개는 찰싹, 그보다 많으면 철썩. 수를 모르면(전부 정리) 찰싹.
fn blow_for(count: Option<usize>) -> Blow {
    match count {
        Some(n) if n <= 5 => Blow { level: "tap", hit_ms: 300, total_ms: 1000, shake: 0.0 },
        Some(n) if n > 50 => Blow { level: "wham", hit_ms: 800, total_ms: 2000, shake: 1.5 },
        _ => Blow { level: "slap", hit_ms: 800, total_ms: 2000, shake: 1.0 },
    }
}

/// 효과를 띄우고 손바닥이 내리치는 순간까지 기다린다. 돌려준 receiver 는 효과 창이 닫히면 끝난다.
/// 효과를 껐거나 창이 안 떠도 정해진 시간 안에 반드시 끝난다.
async fn smash_effect(app: &AppHandle, blow: Blow) -> Option<tokio::sync::oneshot::Receiver<()>> {
    if !cfg_or_default().settings.smash {
        return None;
    }
    if let Some(old) = app.get_webview_window("smash") {
        let _ = old.destroy();
    }
    let (loaded_tx, loaded_rx) = tokio::sync::oneshot::channel::<()>();
    let loaded_tx = Mutex::new(Some(loaded_tx));
    let built = overlay(app, "smash", format!("renderer/smash.html?lang={}&level={}", get_lang(), blow.level), move || {
        if let Some(tx) = loaded_tx.lock().unwrap().take() {
            let _ = tx.send(());
        }
    })
    .build();
    let w = match built {
        Ok(w) => w,
        Err(e) => {
            eprintln!("스매시 효과 창을 만들지 못했습니다: {e}");
            return None;
        }
    };
    // 앱 창이 보이면 그 화면, 아니면 주 화면을 덮는다
    let monitor = app
        .get_webview_window("main")
        .filter(|m| m.is_visible().unwrap_or(false))
        .and_then(|m| m.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let _ = w.set_position(*m.position());
        let _ = w.set_size(*m.size());
    }
    let _ = w.set_ignore_cursor_events(true);

    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel::<()>();
    let closed_tx = Mutex::new(Some(closed_tx));
    w.on_window_event(move |e| {
        if matches!(e, WindowEvent::Destroyed) {
            if let Some(tx) = closed_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
        }
    });
    // 창이 늦게 뜨더라도 정리는 늦어도 1.5초 안에, 창은 늦어도 3초 안에 끝낸다
    let w2 = w.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(3000)).await;
        let _ = w2.destroy();
    });
    let shown = tokio::time::timeout(Duration::from_millis(1500), loaded_rx).await.is_ok();
    if shown {
        let _ = w.show();
        join_all_spaces(&w);
        tokio::time::sleep(Duration::from_millis(blow.hit_ms)).await;
        if blow.shake > 0.0 {
            shake_main(app, blow.shake);
        }
        let w3 = w.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(blow.total_ms - blow.hit_ms + 100)).await;
            let _ = w3.destroy();
        });
    }
    Some(closed_rx)
}

fn shake_main(app: &AppHandle, strength: f64) {
    let Some(win) = app.get_webview_window("main") else { return };
    if !win.is_visible().unwrap_or(false) || win.is_fullscreen().unwrap_or(false) || win.is_maximized().unwrap_or(false) {
        return;
    }
    let (Ok(pos), Ok(scale)) = (win.outer_position(), win.scale_factor()) else { return };
    let start = pos.to_logical::<f64>(scale);
    tauri::async_runtime::spawn(async move {
        for (dx, dy) in [(-16.0, 10.0), (14.0, -9.0), (-10.0, -7.0), (8.0, 6.0), (-4.0, 2.0), (0.0, 0.0)] {
            let _ = win.set_position(LogicalPosition::new(start.x + dx * strength, start.y + dy * strength));
            tokio::time::sleep(Duration::from_millis(45)).await;
        }
    });
}

/// 파일이 가장 많이 들어간 정리 대상 폴더 안에서, 옮긴 자리들의 공통 상위 폴더를 고른다.
/// 예: 문서/사진·이미지와 문서/자료로 옮겼으면 문서, 바탕화면/스크린샷에만 옮겼으면 그 폴더.
fn tidied_folder(cfg: &Config, ops: &[Op]) -> Option<PathBuf> {
    let dirs: Vec<PathBuf> = ops.iter().filter(|o| o.kind == "move").filter_map(|o| Path::new(o.to.as_deref()?).parent().map(Path::to_path_buf)).collect();
    if dirs.is_empty() {
        return None;
    }
    let mut roots: Vec<PathBuf> = cfg.targets.iter().map(organizer::target_root).collect();
    roots.sort_by_key(|r| std::cmp::Reverse(r.as_os_str().len()));
    let mut groups: Vec<(PathBuf, Vec<PathBuf>)> = Vec::new();
    for d in dirs {
        let root = roots.iter().find(|r| d.starts_with(r)).cloned().unwrap_or_else(|| d.clone());
        match groups.iter_mut().find(|(r, _)| *r == root) {
            Some((_, list)) => list.push(d),
            None => groups.push((root, vec![d])),
        }
    }
    // 같은 수면 먼저 나온 쪽 (JS 의 안정 정렬과 같게)
    let mut best = 0;
    for (i, (_, list)) in groups.iter().enumerate() {
        if list.len() > groups[best].1.len() {
            best = i;
        }
    }
    let (root, list) = &groups[best];
    let mut common = list[0].clone();
    for d in list {
        while common != *root && !d.starts_with(&common) {
            match common.parent() {
                Some(p) => common = p.to_path_buf(),
                None => break,
            }
        }
    }
    Some(common)
}

/// 효과를 띄우고, 손바닥이 내리치는 순간 정리한 뒤, 효과 창이 닫힌 다음에 폴더를 연다.
/// `count` 는 치울 파일 수(세기를 정한다). 모르면 None.
async fn smash_and_apply<F>(app: &AppHandle, count: Option<usize>, plan: F) -> Result<Applied, String>
where
    F: FnOnce(&Config) -> Applied + Send + 'static,
{
    let queue = app.state::<SmashQueue>();
    let _turn = queue.0.lock().await;
    let blow = blow_for(count);
    let closed = smash_effect(app, blow).await;
    // 내리치는 순간: 화면은 여기서 소리를 내고 맞은 행을 뺀다 (효과를 꺼도 소리는 따로 켜고 끌 수 있다)
    let _ = app.emit_to("main", "smash:hit", json!({ "level": blow.level, "visual": closed.is_some() }));
    let (cfg, res) = blocking(move || {
        let cfg = load_config()?;
        let res = plan(&cfg);
        Ok((cfg, res))
    })
    .await?;
    // 헛스윙: 하나도 못 치웠으면 효과 창의 글자를 바꾼다
    if !res.results.iter().any(|r| r.ok) {
        if let Some(w) = app.get_webview_window("smash") {
            let _ = w.eval("window.smashMiss && window.smashMiss()");
        }
    }
    let dir = if cfg.settings.open_after { res.run.as_ref().and_then(|r| tidied_folder(&cfg, &r.ops)) } else { None };
    if let Some(rx) = closed {
        let _ = tokio::time::timeout(Duration::from_millis(3500), rx).await;
    }
    if let Some(dir) = dir {
        let _ = app.opener().open_path(dir.to_string_lossy(), None::<&str>);
    }
    Ok(res)
}

/// 트레이의 "지금 전부 정리하기": 폴더가 여러 개여도 기록은 실행 하나로 남는다(한 번에 되돌릴 수 있게).
async fn tidy_all_now(app: AppHandle) {
    let res = smash_and_apply(&app, None, |cfg| apply_all(cfg, "app")).await;
    let total = res.map(|r| r.results.iter().filter(|x| x.ok).count()).unwrap_or(0);
    let _ = app.emit_to("main", "auto:ran", json!({ "count": total }));
    let title = if total > 0 {
        l(&format!("찰싹!! {total}개 치웠어요"), &format!("SMACK!! Tidied {total} files"))
    } else {
        l("치울 게 없네요. 깨끗해요!", "Nothing to tidy. All clean!")
    };
    let _ = app.notification().builder().title(title).show();
}

// ── 잔소리 배너 ───────────────────────────────────────
// 시스템 알림은 권한에 따라 안 뜰 때가 있어서, 화면 위쪽 가운데에 투명 창으로 직접 띄운다.
// 누르면 정리한 폴더(또는 미리보기·기록)를 연다. 4.5초 뒤 저절로 사라지고, 마우스를 올리면 멈춘다.

fn show_nag(app: &AppHandle, title: String, sub: String, action: Option<NagAction>) {
    if let Some(old) = app.get_webview_window("nag") {
        let _ = old.destroy();
    }
    *app.state::<NagFolder>().0.lock().unwrap() = action;
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    // 카드(436×70)에 그림자가 잘리지 않을 만큼 여백을 둔 투명 창
    let (width, height) = (500.0, 130.0);
    let url = format!("renderer/nag.html?title={}&sub={}&lang={}", enc(&title), enc(&sub), get_lang());
    let handle = app.clone();
    let mut builder = overlay(app, "nag", url, move || {
        if let Some(w) = handle.get_webview_window("nag") {
            let _ = w.show();
            join_all_spaces(&w);
        }
    })
    .inner_size(width, height);
    if let Some(m) = monitor {
        let scale = m.scale_factor();
        let area = m.work_area();
        let x = area.position.x as f64 / scale + (area.size.width as f64 / scale - width) / 2.0;
        let y = area.position.y as f64 / scale;
        builder = builder.position(x, y);
    }
    match builder.build() {
        Ok(w) => {
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(NAG_MAX_MS)).await;
                let _ = w.destroy();
            });
        }
        Err(e) => eprintln!("잔소리 배너를 띄우지 못했습니다: {e}"),
    }
}

/// 자동 정리 뒤 배너 제목. 치운 양에 따라 말이 달라진다.
fn auto_title(count: usize) -> String {
    match count {
        0..=4 => l("엄마가 또 치웠다", "Mom tidied up again"),
        5..=19 => l("으이구!! 폴더 청소 좀 하랬지!!", "Ugh!! Didn’t I tell you to clean your folders!!"),
        _ => l("이게 다 뭐니", "What is all this?"),
    }
}

fn on_auto_run(app: &AppHandle, target_id: &str, result: &Applied) {
    let ok: Vec<&organizer::ApplyResult> = result.results.iter().filter(|r| r.ok).collect();
    let _ = app.emit_to("main", "auto:ran", json!({ "targetId": target_id, "count": ok.len() }));
    update_tray(app);
    let cfg = cfg_or_default();
    if ok.is_empty() || !cfg.settings.notify {
        return;
    }
    let names = ok.iter().take(2).map(|r| r.name.as_str()).collect::<Vec<_>>().join(", ");
    let more = if ok.len() > 2 { l(&format!(" 외 {}개", ok.len() - 2), &format!(" and {} more", ok.len() - 2)) } else { String::new() };
    let root = cfg.targets.iter().find(|t| t.id == target_id).map(organizer::target_root).unwrap_or_default();
    let folder = result.run.as_ref().and_then(|r| tidied_folder(&cfg, &r.ops)).unwrap_or_else(|| root.clone());
    let where_ = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let n = ok.len();
    let sub = l(&format!("{where_}에서 {n}개 치웠어요 · {names}{more}"), &format!("Tidied {n} in {where_} · {names}{more}"));
    let handle = app.clone();
    let folder = folder.to_string_lossy().into_owned();
    let _ = app.run_on_main_thread(move || show_nag(&handle, auto_title(n), sub, Some(NagAction::Folder(folder))));
}

// ── 치우기 전 잔소리, 주간 성적표 ─────────────────────
// 자동 정리를 켜지 않은 폴더에 규칙에 걸리는 파일이 쌓이면 단계별로 한 번씩 잔소리한다.
// 폴더마다 하루 한 번까지만 뜨고, 설정에서 끌 수 있다.

fn nag_level(count: usize) -> u8 {
    NAG_STEPS.iter().filter(|s| count >= **s).count() as u8
}

fn nag_title(level: u8) -> String {
    match level {
        1 => l("방 좀 치워라!", "Clean your room!"),
        2 => l("으이구, 발 디딜 틈이 없네", "Ugh, there’s nowhere left to step"),
        _ => l("셋 센다. 하나…", "I’m counting to three. One…"),
    }
}

/// 지난번(단계, 날짜)과 견줘 지금 잔소리할지: 단계가 올랐고 오늘 아직 안 했을 때만.
fn should_nag(prev: Option<(u8, &str)>, level: u8, today: &str) -> bool {
    level > 0 && prev.is_none_or(|(was, day)| level > was && day != today)
}

fn state_path() -> PathBuf {
    config::config_dir().join("state.json")
}

fn load_state() -> Value {
    std::fs::read_to_string(state_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

fn fmt_bytes(n: u64) -> String {
    let n = n as f64;
    if n >= 1.0e9 {
        format!("{:.1}GB", n / 1.0e9)
    } else if n >= 1.0e6 {
        format!("{:.0}MB", n / 1.0e6)
    } else {
        format!("{:.0}KB", (n / 1.0e3).max(1.0))
    }
}

fn folder_name(t: &config::Target) -> String {
    Path::new(&expand_home(&t.path)).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// 10분마다 한 번(자동 정리 스레드에서) 불린다.
fn on_sweep(app: &AppHandle) {
    let cfg = cfg_or_default();
    if !cfg.settings.notify {
        return;
    }
    let mut state = load_state();
    let before = state.clone();
    let now = chrono::Local::now();
    let today = now.format("%Y-%m-%d").to_string();
    let mut banner: Option<(String, String, NagAction)> = None;

    if cfg.settings.nag {
        for t in cfg.targets.iter().filter(|t| !t.auto) {
            let count = organizer::count_matching(&cfg, t);
            let level = nag_level(count);
            let prev = state["nag"][&t.id].clone();
            let was = prev["level"].as_u64().map(|v| (v as u8, prev["day"].as_str().unwrap_or("")));
            if should_nag(was, level, &today) && banner.is_none() {
                let name = folder_name(t);
                let sub = l(&format!("{name}에 치울 파일이 {count}개 쌓였어요. 누르면 미리 볼 수 있어요."), &format!("{count} files to tidy in {name}. Click to preview."));
                banner = Some((nag_title(level), sub, NagAction::Preview(t.id.clone())));
                state["nag"][&t.id] = json!({ "level": level, "day": today });
            } else if was.is_some_and(|(w, _)| level < w) {
                // 치워서 줄었으면 단계를 내린다. 다시 쌓이면 다시 말한다.
                state["nag"][&t.id] = json!({ "level": level, "day": prev["day"] });
            }
        }
    }

    // 일요일 저녁에 한 번: 이번 주에 치운 양
    use chrono::{Datelike, Timelike};
    let week = now.format("%G-W%V").to_string();
    if cfg.settings.weekly && banner.is_none() && now.weekday() == chrono::Weekday::Sun && now.hour() >= 18 && state["week"].as_str() != Some(&week) {
        state["week"] = json!(week);
        let stats = history::week_stats(config::now_ms());
        let (moved, trashed, bytes) = (stats["moved"].as_u64().unwrap_or(0), stats["trashed"].as_u64().unwrap_or(0), stats["trashBytes"].as_u64().unwrap_or(0));
        if moved + trashed > 0 {
            let total = moved + trashed;
            let left: Vec<String> =
                cfg.targets.iter().map(|t| (folder_name(t), organizer::count_matching(&cfg, t))).filter(|(_, n)| *n > 0).map(|(name, n)| l(&format!("{name}에 아직 {n}개"), &format!("{n} left in {name}"))).take(2).collect();
            let mut parts = vec![];
            if trashed > 0 {
                parts.push(l(&format!("휴지통 {}", fmt_bytes(bytes)), &format!("{} to Trash", fmt_bytes(bytes))));
            }
            parts.extend(left);
            let title = l(&format!("이번 주 {total}개 치웠어요"), &format!("{total} files tidied this week"));
            banner = Some((title, parts.join(" · "), NagAction::History));
        }
    }

    if state != before {
        let _ = config::write_json_atomic(&state_path(), &state);
    }
    if let Some((title, sub, action)) = banner {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || show_nag(&handle, title, sub, Some(action)));
    }
}

/// 설정 파일을 지켜본다. MCP 서버(다른 프로세스)가 규칙을 넣으면 화면과 자동 정리에 바로 반영한다.
fn spawn_config_watch(app: AppHandle) {
    use notify::Watcher;
    thread::spawn(move || {
        let dir = config::config_dir();
        let _ = std::fs::create_dir_all(&dir);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let made = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if res.is_ok_and(|e| e.paths.iter().any(|p| p.file_name().is_some_and(|n| n == "config.json"))) {
                let _ = tx.send(());
            }
        });
        let Ok(mut watcher) = made else { return };
        if watcher.watch(&dir, notify::RecursiveMode::NonRecursive).is_err() {
            return;
        }
        while rx.recv().is_ok() {
            // 저장은 임시 파일 → 이름 바꾸기라 이벤트가 몇 개 이어진다. 잠깐 모아서 한 번만 본다.
            thread::sleep(Duration::from_millis(300));
            while rx.try_recv().is_ok() {}
            let rev = config::config_rev();
            let known = app.state::<ConfigRev>();
            let mut last = known.0.lock().unwrap();
            if rev.is_empty() || *last == rev {
                continue; // 앱이 스스로 저장한 것
            }
            *last = rev;
            drop(last);
            if let Ok(cfg) = load_config() {
                apply_settings(&app, &cfg);
                let _ = app.emit_to("main", "config:changed", ());
            }
        }
    });
}

fn remember_rev(app: &AppHandle) -> String {
    let rev = config::config_rev();
    *app.state::<ConfigRev>().0.lock().unwrap() = rev.clone();
    rev
}

// ── 트레이 ────────────────────────────────────────────

fn tray_menu(app: &AppHandle, cfg: &Config) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    let mut b = MenuBuilder::new(app);
    if let Some((u, _)) = app.state::<PendingUpdate>().0.lock().unwrap().as_ref() {
        let text = l(&format!("새 버전 v{} 설치하고 다시 시작", u.version), &format!("Install v{} and restart", u.version));
        b = b.item(&MenuItem::with_id(app, "update", text, true, None::<&str>)?).separator();
    }
    b = b.item(&MenuItem::with_id(app, "open", l(&format!("{APP_NAME} 열기"), &format!("Open {APP_NAME}")), true, None::<&str>)?).separator();
    for t in &cfg.targets {
        let name = Path::new(&expand_home(&t.path)).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let text = l(&format!("{name} 자동 정리"), &format!("Auto-tidy {name}"));
        b = b.item(&CheckMenuItem::with_id(app, format!("auto:{}", t.id), text, true, t.auto, None::<&str>)?);
    }
    let info = app.state::<UpdateStatus>().0.lock().unwrap().clone();
    let version = format!("{APP_NAME} v{}", app.package_info().version);
    let check = match (&info.latest, info.checking, info.ready) {
        (_, _, true) => None, // 설치 항목이 맨 위에 있다
        (Some(v), true, _) => Some((l(&format!("새 버전 v{v} 받는 중…"), &format!("Downloading v{v}…")), false)),
        (None, true, _) => Some((l("업데이트 확인 중…", "Checking for updates…"), false)),
        _ => Some((l("업데이트 확인…", "Check for updates…"), true)),
    };
    b = b
        .item(&MenuItem::with_id(app, "tidy", l("지금 전부 정리하기", "Tidy everything now"), true, None::<&str>)?)
        .separator()
        .item(&MenuItem::with_id(app, "version", version, false, None::<&str>)?);
    if let Some((text, enabled)) = check {
        b = b.item(&MenuItem::with_id(app, "update_check", text, enabled, None::<&str>)?);
    }
    b.separator().item(&MenuItem::with_id(app, "quit", l("종료", "Quit"), true, None::<&str>)?).build()
}

fn update_tray(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(tray) = handle.tray_by_id("main") else { return };
        let cfg = cfg_or_default();
        if let Ok(menu) = tray_menu(&handle, &cfg) {
            let _ = tray.set_menu(Some(menu));
        }
        let pending = handle.state::<PendingUpdate>().0.lock().unwrap().as_ref().map(|(u, _)| u.version.clone());
        let tip = match pending {
            Some(v) => format!("{APP_NAME} · {}", l(&format!("새 버전 v{v}"), &format!("v{v} available"))),
            None if any_auto(&cfg) => format!("{APP_NAME} · {}", l("자동 정리 중", "auto-tidy on")),
            None => APP_NAME.into(),
        };
        let _ = tray.set_tooltip(Some(tip));
    });
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let icon = if cfg!(target_os = "macos") {
        tauri::image::Image::from_bytes(include_bytes!("../../assets/trayTemplate@2x.png"))?
    } else {
        tauri::image::Image::from_bytes(include_bytes!("../../assets/tray-color.png"))?
    };
    let menu = tray_menu(app, &cfg_or_default())?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip(APP_NAME)
        .menu(&menu)
        // macOS 는 왼쪽 클릭에 메뉴, Windows 는 왼쪽 클릭에 창 (Electron 때와 같게)
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_string();
            match id.as_str() {
                "open" => show_window(app, None),
                "tidy" => {
                    tauri::async_runtime::spawn(tidy_all_now(app.clone()));
                }
                "update" => install_update(app),
                // 결과는 설정 탭의 앱 정보 카드에서 보여 준다 (메뉴 막대 앱은 대화상자가 뒤로 숨기 쉽다)
                "update_check" => {
                    show_window(app, Some("settings"));
                    let handle = app.clone();
                    tauri::async_runtime::spawn(async move {
                        check_update(&handle).await;
                    });
                }
                "quit" => {
                    app.state::<Quitting>().0.store(true, Ordering::SeqCst);
                    app.exit(0);
                }
                other => {
                    if let Some(tid) = other.strip_prefix("auto:") {
                        let mut cfg = cfg_or_default();
                        let Some(t) = cfg.targets.iter_mut().find(|t| t.id == tid) else { return };
                        if !t.auto {
                            // 켤 때는 바로 실행하지 않고, 지금 쌓인 파일을 어떻게 치울지 화면에서 먼저 보여 준다
                            update_tray(app); // 메뉴의 체크 표시를 원래대로
                            show_window(app, Some("settings"));
                            let _ = app.emit_to("main", "auto:confirm", tid);
                            return;
                        }
                        t.auto = false;
                        match save_config(&cfg) {
                            Ok(saved) => {
                                remember_rev(app);
                                apply_settings(app, &saved);
                                let _ = app.emit_to("main", "config:changed", ());
                            }
                            Err(e) => eprintln!("설정을 저장하지 못했습니다: {e}"),
                        }
                    }
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if cfg!(target_os = "macos") {
                return;
            }
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(tray.app_handle(), None);
            }
        })
        .build(app)?;
    Ok(())
}

// ── 자동 업데이트 ─────────────────────────────────────

fn spawn_update_checker(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    thread::spawn(move || {
        thread::sleep(UPDATE_FIRST_CHECK);
        loop {
            // 저절로 찾아 받아 둔 새 버전은 한 번 알린다 (받아 두면 설치할 때까지 다시 확인하지 않는다)
            if tauri::async_runtime::block_on(check_update(&app)) {
                notify_update(&app);
            }
            thread::sleep(UPDATE_INTERVAL);
        }
    });
}

fn update_snapshot(app: &AppHandle) -> Value {
    let info = app.state::<UpdateStatus>().0.lock().unwrap().clone();
    let mut v = json!(info);
    v["current"] = json!(app.package_info().version.to_string());
    v
}

/// 업데이트 상태를 바꾸고 설정 화면과 트레이에 알린다
fn set_update(app: &AppHandle, f: impl FnOnce(&mut UpdateInfo)) {
    f(&mut app.state::<UpdateStatus>().0.lock().unwrap());
    let _ = app.emit_to("main", "update:state", update_snapshot(app));
    update_tray(app);
}

/// 새 버전이 있으면 받아 둔다. 이번 호출에서 새로 받아 뒀으면 true
async fn check_update(app: &AppHandle) -> bool {
    // 잠금은 await 전에 풀어야 spawn 할 수 있다(Send)
    let pending = app.state::<PendingUpdate>().0.lock().unwrap().is_some();
    if pending {
        return false; // 이미 받아 둔 것이 있으면 설치를 기다린다
    }
    {
        let status = app.state::<UpdateStatus>();
        let mut s = status.0.lock().unwrap();
        if s.checking {
            return false; // 트레이·설정 화면·주기 확인이 겹치면 앞의 것을 기다린다
        }
        s.checking = true;
    }
    set_update(app, |s| s.error = false);
    let res: Result<Option<(Update, Vec<u8>)>, String> = async {
        let Some(update) = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        let v = update.version.clone();
        set_update(app, |s| s.latest = Some(v));
        let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
        Ok(Some((update, bytes)))
    }
    .await;
    match res {
        Ok(Some(pending)) => {
            *app.state::<PendingUpdate>().0.lock().unwrap() = Some(pending);
            set_update(app, |s| {
                s.checking = false;
                s.checked = true;
                s.ready = true;
            });
            true
        }
        Ok(None) => {
            set_update(app, |s| {
                s.checking = false;
                s.checked = true;
                s.latest = None;
            });
            false
        }
        Err(e) => {
            eprintln!("업데이트 확인 실패: {e}");
            set_update(app, |s| {
                s.checking = false;
                s.checked = true;
                s.error = true;
                s.latest = None;
            });
            false
        }
    }
}

fn notify_update(app: &AppHandle) {
    let Some(v) = app.state::<UpdateStatus>().0.lock().unwrap().latest.clone() else { return };
    let place = if cfg!(target_os = "macos") { l("메뉴 막대", "menu bar") } else { l("알림 영역", "notification area") };
    let _ = app
        .notification()
        .builder()
        .title(l(&format!("{APP_NAME} 새 버전(v{v})을 받아 뒀어요"), &format!("{APP_NAME} v{v} is ready")))
        .body(l(&format!("{place}의 {APP_NAME} 메뉴나 설정에서 설치하면 다시 시작해요."), &format!("Install it from the {APP_NAME} {place} menu or Settings. The app restarts.")))
        .show();
}

fn install_update(app: &AppHandle) {
    let Some((update, bytes)) = app.state::<PendingUpdate>().0.lock().unwrap().take() else { return };
    match update.install(bytes) {
        // Windows 는 설치 프로그램이 앱을 닫고 새 버전을 띄운다. macOS 는 앱 묶음을 바꾼 뒤 다시 시작한다
        Ok(()) => {
            app.state::<Quitting>().0.store(true, Ordering::SeqCst);
            app.restart();
        }
        Err(e) => {
            // 받아 둔 것을 버렸으니 메뉴를 되돌리고, 다음 확인 때 다시 받는다
            eprintln!("업데이트 설치 실패: {e}");
            set_update(app, |s| {
                s.ready = false;
                s.latest = None;
                s.error = true;
            });
        }
    }
}

/// Electron 시절 등록(또는 앱을 옮기기 전 경로)으로 남은 smash MCP 등록을 지금 실행 파일로 바꾼다.
/// 디스크 이미지에서 바로 띄웠거나(App Translocation) 개발 빌드면 건드리지 않는다.
fn spawn_registration_fix() {
    if cfg!(debug_assertions) {
        return;
    }
    let launch = Launch::current();
    if launch.command.is_empty() || launch.command.contains("/AppTranslocation/") || launch.command.starts_with("/Volumes/") {
        return;
    }
    thread::spawn(move || {
        let changed = claude::refresh_registrations(&launch);
        if !changed.is_empty() {
            eprintln!("MCP 등록을 새 실행 파일로 바꿨습니다: {}", changed.join(", "));
        }
    });
}

// ── 화면에서 부르는 명령 ──────────────────────────────

#[tauri::command]
fn config_get() -> Result<Value, String> {
    let cfg = load_config()?;
    Ok(json!({
        "config": cfg,
        "configPath": label(&config::config_path().to_string_lossy()),
        "home": config::home().to_string_lossy(),
        "sep": std::path::MAIN_SEPARATOR.to_string(),
        "categories": config::local_categories(),
        "lang": get_lang(),
        "platform": platform(),
        "rev": config::config_rev(),
        "canRestoreTrash": trash::can_restore(),
    }))
}

#[tauri::command]
fn config_save(app: AppHandle, cfg: Value) -> Result<Value, String> {
    let saved = save_config_value(&cfg)?;
    let rev = remember_rev(&app);
    apply_settings(&app, &saved);
    Ok(json!({ "config": saved, "rev": rev }))
}

/// `shallow`: 하위 폴더까지 정리가 켜져 있어도 바로 아래 파일만 본다(자동 정리가 보는 범위).
#[tauri::command]
async fn scan(target_id: Option<String>, shallow: Option<bool>) -> Result<Value, String> {
    blocking(move || {
        let res = scan_target_with(&load_config()?, target_id.as_deref(), !shallow.unwrap_or(false))?;
        let unmatched: Vec<Value> = res
            .unmatched
            .iter()
            .map(|f| {
                let mut v = json!(f);
                v["srcLabel"] = json!(label(&f.path));
                v
            })
            .collect();
        Ok(json!({
            "targetId": res.target_id,
            "root": res.root,
            "rootLabel": label(&res.root),
            "items": decorate(&res.items),
            "unmatched": unmatched,
            "blocked": decorate(&res.blocked),
            "truncated": res.truncated,
        }))
    })
    .await
}

#[tauri::command]
async fn folders() -> Result<Value, String> {
    blocking(|| Ok(json!(organizer::folder_tree(&load_config()?, 3).iter().map(|f| label(f)).collect::<Vec<_>>()))).await
}

#[tauri::command]
async fn rule_test(target_id: String, rule: Value) -> Result<Value, String> {
    blocking(move || to_value(organizer::test_rule(&load_config()?, &target_id, &rule)?)).await
}

fn target_root_of(cfg: &Config, target_id: &str) -> Result<String, String> {
    find_target(cfg, Some(target_id))
        .map(|t| organizer::target_root(t).to_string_lossy().into_owned())
        .ok_or_else(|| l("폴더를 찾을 수 없습니다", "Folder not found"))
}

#[tauri::command]
async fn rule_from_text(target_id: String, text: String) -> Result<Value, String> {
    blocking(move || {
        let cfg = load_config()?;
        let target = find_target(&cfg, Some(&target_id)).ok_or_else(|| l("폴더를 찾을 수 없습니다", "Folder not found"))?;
        let text: String = text.chars().take(500).collect();
        claude::rules_proposal(&cfg, target, &text)
    })
    .await
}

/// 미리보기 계획의 사진·동영상을 Claude 에 썸네일로 보여 주고 소분류를 다시 정한다.
#[tauri::command]
async fn ai_group_media(items: Vec<Item>) -> Result<Value, String> {
    blocking(move || {
        let cfg = load_config()?;
        let assign = claude::group_media_by_content(&cfg, &items)?;
        let grouped = assign.len();
        let items = organizer::regroup(&cfg, items, &assign);
        Ok(json!({ "items": decorate(&items), "grouped": grouped }))
    })
    .await
}

#[tauri::command]
async fn apply(app: AppHandle, items: Vec<Item>) -> Result<Value, String> {
    let count = items.len();
    let progress = app.clone();
    let res = smash_and_apply(&app, Some(count), move |cfg| {
        let mut rec = Recorder::begin("app");
        // 오래 걸리는 정리는 몇 개째인지 화면에 알려 준다 (너무 잦지 않게)
        let mut last = std::time::Instant::now();
        rec.progress = Some(Box::new(move |done, total| {
            if done == total || last.elapsed() >= Duration::from_millis(120) {
                last = std::time::Instant::now();
                let _ = progress.emit_to("main", "apply:progress", json!({ "done": done, "total": total }));
            }
        }));
        let results = apply_into(cfg, items, &mut rec);
        let (run, history_error) = rec.finish();
        Applied { results, run, history_error }
    })
    .await?;
    to_value(res)
}

/// `run_id` 가 없으면 아직 돌릴 것이 남은 가장 최근 실행을 되돌린다.
#[tauri::command]
async fn undo(run_id: Option<String>) -> Result<Value, String> {
    blocking(move || Ok(organizer::undo_run(run_id.as_deref()))).await
}

#[tauri::command]
fn open_trash(app: AppHandle) -> Result<(), String> {
    if cfg!(target_os = "windows") {
        return std::process::Command::new("explorer.exe").arg("shell:RecycleBinFolder").spawn().map(|_| ()).map_err(|e| e.to_string());
    }
    let dir = if cfg!(target_os = "macos") { config::home().join(".Trash") } else { config::home().join(".local/share/Trash/files") };
    app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
async fn history() -> Result<Value, String> {
    blocking(|| {
        let runs: Vec<Value> = organizer::load_history()
            .into_iter()
            .rev()
            .map(|r| {
                let mut v = json!(r);
                v["left"] = json!(r.retryable());
                v["failed"] = json!(r.failed.iter().map(|f| json!({ "file": label(&f.file), "reason": f.reason })).collect::<Vec<_>>());
                v["ops"] = json!(r
                    .ops
                    .iter()
                    .map(|o| {
                        let mut ov = json!(o);
                        ov["fromLabel"] = json!(label(&o.from));
                        ov["toLabel"] = json!(label(o.to.as_deref().unwrap_or("")));
                        ov
                    })
                    .collect::<Vec<_>>());
                v
            })
            .collect();
        Ok(json!({ "runs": runs, "week": history::week_stats(config::now_ms()) }))
    })
    .await
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnmatchedIn {
    name: String,
    path: String,
    #[serde(default)]
    size: f64,
    #[serde(default)]
    mtime_ms: f64,
}

#[tauri::command]
async fn ai_suggest(target_id: String, unmatched: Vec<UnmatchedIn>) -> Result<Value, String> {
    blocking(move || {
        let cfg = load_config()?;
        let root = target_root_of(&cfg, &target_id)?;
        let files: Vec<organizer::FileInfo> =
            unmatched.into_iter().map(|u| organizer::FileInfo { name: u.name, path: u.path, size: u.size, mtime_ms: u.mtime_ms, received_ms: None }).collect();
        let mut res = claude::suggest_placements(&cfg, &root, &files)?;
        let items: Vec<Item> = serde_json::from_value(res["items"].take()).map_err(|e| e.to_string())?;
        res["items"] = decorate(&items);
        Ok(res)
    })
    .await
}

#[tauri::command]
async fn integration_status() -> Result<Value, String> {
    blocking(|| Ok(claude::integration_status(&Launch::current()))).await
}

#[tauri::command]
async fn integration_claude_code(location_ids: Option<Vec<String>>) -> Result<Value, String> {
    blocking(move || Ok(json!(claude::register_claude_code(&Launch::current(), location_ids)))).await
}

#[tauri::command]
async fn integration_claude_desktop() -> Result<Value, String> {
    blocking(|| Ok(json!(claude::register_claude_desktop(&Launch::current())?))).await
}

#[tauri::command]
async fn dialog_folder(app: AppHandle, default_path: Option<String>) -> Result<Value, String> {
    blocking(move || {
        let mut d = app.dialog().file();
        if let Some(p) = default_path.filter(|p| !p.is_empty()) {
            d = d.set_directory(expand_home(&p));
        }
        if let Some(w) = app.get_webview_window("main") {
            d = d.set_parent(&w);
        }
        Ok(match d.blocking_pick_folder() {
            Some(p) => json!(p.into_path().map_err(|e| e.to_string())?.to_string_lossy()),
            None => Value::Null,
        })
    })
    .await
}

#[tauri::command]
fn shell_show(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn shell_open_url(app: AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Ok(());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_status(app: AppHandle) -> Value {
    update_snapshot(&app)
}

#[tauri::command]
async fn update_check(app: AppHandle) -> Result<Value, String> {
    check_update(&app).await;
    Ok(update_snapshot(&app))
}

#[tauri::command]
fn update_install(app: AppHandle) {
    install_update(&app);
}

/// 잔소리 배너를 눌렀을 때: 정리한 폴더를 열거나, 앱 창에서 미리보기·기록을 보여 준다
#[tauri::command]
fn nag_open(app: AppHandle) {
    let action = app.state::<NagFolder>().0.lock().unwrap().take();
    match action {
        Some(NagAction::Folder(f)) => {
            let _ = app.opener().open_path(f, None::<&str>);
        }
        Some(NagAction::Preview(target_id)) => {
            show_window(&app, Some("preview"));
            let _ = app.emit_to("main", "ui:target", target_id);
        }
        Some(NagAction::History) => show_window(&app, Some("history")),
        None => {}
    }
    nag_close(app);
}

/// 배너가 다 보여 주고 스스로 닫을 때
#[tauri::command]
fn nag_close(app: AppHandle) {
    if let Some(w) = app.get_webview_window("nag") {
        let _ = w.destroy();
    }
}

// ── 시작 ──────────────────────────────────────────────

pub fn run() {
    let app = tauri::Builder::default()
        // 이미 떠 있으면 새로 띄우지 않고 그 창을 보여 준다 (가장 먼저 등록해야 한다)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_window(app, None)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Auto(Mutex::new(None)))
        .manage(SmashQueue(tokio::sync::Mutex::new(())))
        .manage(NagFolder(Mutex::new(None)))
        .manage(ConfigRev::default())
        .manage(Quitting(AtomicBool::new(false)))
        .manage(PendingUpdate::default())
        .manage(UpdateStatus::default())
        .invoke_handler(tauri::generate_handler![
            config_get,
            config_save,
            scan,
            folders,
            rule_test,
            rule_from_text,
            apply,
            undo,
            history,
            ai_suggest,
            ai_group_media,
            integration_status,
            integration_claude_code,
            integration_claude_desktop,
            dialog_folder,
            shell_show,
            shell_open_url,
            nag_open,
            nag_close,
            open_trash,
            update_status,
            update_check,
            update_install,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            // 처음 실행이면 기본 설정(폴더·규칙 이름)도 OS 언어로 만들어진다
            set_lang(resolve_lang(None, None));
            let cfg = cfg_or_default();
            sync_lang(&cfg);
            let start_hidden = std::env::args().any(|a| a == "--hidden");
            build_tray(&handle)?;
            create_main(&handle, !start_hidden)?;
            if start_hidden {
                set_dock(&handle, false);
            }
            let runner = handle.clone();
            let sweeper = handle.clone();
            let auto = AutoTidy::start(Box::new(move |id, res| on_auto_run(&runner, id, res)), Box::new(move || on_sweep(&sweeper)));
            *handle.state::<Auto>().0.lock().unwrap() = Some(auto);
            apply_settings(&handle, &cfg);
            remember_rev(&handle);
            spawn_config_watch(handle.clone());
            spawn_update_checker(handle.clone());
            spawn_registration_fix();
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Smash! 를 시작하지 못했습니다");

    // 창을 닫으면 숨기기만 하므로(create_main) 앱은 트레이에서 계속 돈다. 끝내기는 트레이 메뉴의 '종료'.
    app.run(|app, event| match event {
        RunEvent::Exit => {
            if let Some(a) = app.state::<Auto>().0.lock().unwrap().as_ref() {
                a.stop();
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_window(app, None),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidied_folder_picks_common_parent() {
        let mut cfg = config::default_config();
        cfg.targets.truncate(1);
        cfg.targets[0].path = "/t/docs".into();
        let op = |to: &str| Op { kind: "move".into(), from: "/x".into(), to: Some(to.into()), ..Default::default() };
        let ops = vec![op("/t/docs/사진/a.png"), op("/t/docs/자료/b.pdf")];
        assert_eq!(tidied_folder(&cfg, &ops), Some(PathBuf::from("/t/docs")));
        let ops = vec![op("/t/docs/shots/a.png"), op("/t/docs/shots/b.png")];
        assert_eq!(tidied_folder(&cfg, &ops), Some(PathBuf::from("/t/docs/shots")));
        assert_eq!(tidied_folder(&cfg, &[]), None);
    }

    #[test]
    fn blow_scales_with_file_count() {
        assert_eq!(blow_for(Some(1)).level, "tap");
        assert_eq!(blow_for(Some(5)).level, "tap");
        assert_eq!(blow_for(Some(6)).level, "slap");
        assert_eq!(blow_for(Some(50)).level, "slap");
        assert_eq!(blow_for(Some(51)).level, "wham");
        assert_eq!(blow_for(None).level, "slap");
        assert_eq!(blow_for(Some(2)).shake, 0.0, "몇 개 안 되면 창을 흔들지 않는다");
        assert!(blow_for(Some(500)).shake > blow_for(Some(20)).shake);
        for n in [Some(1), Some(20), Some(500), None] {
            let b = blow_for(n);
            assert!(b.hit_ms < b.total_ms && b.total_ms <= 2000);
        }
    }

    #[test]
    fn nag_steps_and_once_a_day() {
        assert_eq!([0, 19, 20, 59, 60, 149, 150, 9999].map(nag_level), [0, 0, 1, 1, 2, 2, 3, 3]);
        assert!(!should_nag(None, 0, "2026-11-10"), "쌓인 게 없으면 말하지 않는다");
        assert!(should_nag(None, 1, "2026-11-10"));
        assert!(!should_nag(Some((1, "2026-11-09")), 1, "2026-11-10"), "같은 단계로는 다시 말하지 않는다");
        assert!(should_nag(Some((1, "2026-11-09")), 2, "2026-11-10"));
        assert!(!should_nag(Some((1, "2026-11-10")), 2, "2026-11-10"), "하루에 한 번까지");
        assert_eq!(fmt_bytes(1_200_000_000), "1.2GB");
        assert_eq!(fmt_bytes(3_400_000), "3MB");
    }

    #[test]
    fn query_encoding() {
        assert_eq!(enc("a b&c=다"), "a%20b%26c%3D%EB%8B%A4");
    }
}
