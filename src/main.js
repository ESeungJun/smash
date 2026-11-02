'use strict';
const os = require('os');
const fs = require('fs');
const path = require('path');
const { app, BrowserWindow, ipcMain, dialog, shell, Tray, Menu, Notification, nativeImage, nativeTheme, screen } = require('electron');
const { loadConfig, saveConfig, configPath, contractHome, expandHome, localCategories } = require('./core/config');
const { scanTarget, applyPlan, undoLast, loadHistory, findTarget, testRule, missingDirs, folderTree } = require('./core/organizer');
const { suggestPlacements, integrationStatus, registerClaudeCode, registerClaudeDesktop, ruleFromText, MCP_NAME } = require('./core/claude');
const { AutoTidy } = require('./core/watcher');
const { L, getLang, setLang, resolveLang } = require('./core/lang');

const APP_NAME = 'Smash!';
app.setName(APP_NAME);
if (process.platform === 'win32') app.setAppUserModelId('dev.haul.smash');

const assetPath = (name) => path.join(__dirname, '..', 'assets', name);

let win = null;
let tray = null;
let quitting = false;

// Claude가 이 앱을 MCP 서버로 띄울 때 쓰는 명령. Electron 실행 파일을 Node처럼 돌린다.
function mcpLaunch() {
  const script = app.isPackaged
    ? path.join(process.resourcesPath, 'app.asar.unpacked', 'src', 'mcp', 'server.js')
    : path.join(__dirname, 'mcp', 'server.js');
  return { command: process.execPath, args: [script], env: { ELECTRON_RUN_AS_NODE: '1' } };
}

const label = (p) => (p ? contractHome(p) : '');
const decorate = (items) =>
  items.map((it) => ({
    ...it,
    srcLabel: label(it.src),
    destLabel: label(it.dest),
    duplicateLabel: label(it.duplicateOf),
    newDirs: it.action === 'move' && it.dest ? missingDirs(it.dest) : [],
  }));

const anyAuto = (cfg = loadConfig()) => cfg.targets.some((t) => t.auto);

// ---------- 자동 정리 ----------

const auto = new AutoTidy({
  load: loadConfig,
  onRun(target, result) {
    const { results } = result;
    const ok = results.filter((r) => r.ok);
    win?.webContents.send('auto:ran', { targetId: target.id, count: ok.length });
    updateTray();
    if (!ok.length || !loadConfig().settings.notify) return;
    const names = ok.slice(0, 2).map((r) => r.name).join(', ');
    const more = ok.length > 2 ? L(` 외 ${ok.length - 2}개`, ` and ${ok.length - 2} more`) : '';
    const folder = tidiedFolder(loadConfig(), result.run?.ops || []) || target.root;
    const where = path.basename(target.root);
    showNag(L(`${where}에서 ${ok.length}개 치웠어요 · ${names}${more}`, `Tidied ${ok.length} in ${where} · ${names}${more}`), folder);
  },
  onError(target, err) {
    console.error(`[auto] ${target.root}: ${err.message}`);
  },
});

// 설정의 언어(system이면 OS 언어)를 main 쪽 문구에 적용한다. 화면에는 config:get으로 넘긴다.
const syncLang = (cfg) => setLang(resolveLang(cfg.settings.language, app.getLocale()));

function applySettings(cfg) {
  syncLang(cfg);
  auto.sync(cfg);
  // 값이 바뀔 때만 건드린다. 매번 부르면 서명 안 된 빌드에서 권한 오류가 로그에 쌓인다.
  if (app.isPackaged && process.platform !== 'linux' && app.getLoginItemSettings().openAtLogin !== cfg.settings.openAtLogin) {
    app.setLoginItemSettings({ openAtLogin: cfg.settings.openAtLogin, openAsHidden: true, args: ['--hidden'] });
  }
  updateTray();
}

// ---------- 창·트레이 ----------

function createWindow({ show = true } = {}) {
  win = new BrowserWindow({
    width: 1180,
    height: 800,
    minWidth: 860,
    minHeight: 580,
    title: APP_NAME,
    show,
    icon: assetPath('icon-256.png'),
    backgroundColor: nativeTheme.shouldUseDarkColors ? '#111111' : '#f4f3f1',
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  });
  win.removeMenu();
  win.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https:\/\//.test(url)) shell.openExternal(url);
    return { action: 'deny' };
  });
  win.webContents.on('will-navigate', (e) => e.preventDefault());
  // 자동 정리가 켜져 있으면 창을 닫아도 트레이에서 계속 지켜본다.
  win.on('close', (e) => {
    if (!quitting && anyAuto()) {
      e.preventDefault();
      win.hide();
      if (process.platform === 'darwin') app.dock?.hide();
    }
  });
  win.on('closed', () => {
    win = null;
  });
  win.loadFile(path.join(__dirname, 'renderer', 'index.html'));
  setupCapture(win);
}

function showWindow(tab) {
  if (process.platform === 'darwin') app.dock?.show();
  if (!win) createWindow();
  win.show();
  win.focus();
  if (tab) win.webContents.send('ui:tab', tab);
}

function updateTray() {
  if (!tray) return;
  const cfg = loadConfig();
  const autoOn = anyAuto(cfg);
  tray.setToolTip(autoOn ? `${APP_NAME} — ${L('자동 정리 중', 'auto-tidy on')}` : APP_NAME);
  tray.setContextMenu(
    Menu.buildFromTemplate([
      { label: L(`${APP_NAME} 열기`, `Open ${APP_NAME}`), click: () => showWindow() },
      { type: 'separator' },
      ...cfg.targets.map((t) => ({
        label: L(`${path.basename(expandHome(t.path))} 자동 정리`, `Auto-tidy ${path.basename(expandHome(t.path))}`),
        type: 'checkbox',
        checked: t.auto,
        click: (item) => {
          const next = loadConfig();
          const target = next.targets.find((x) => x.id === t.id);
          if (target) target.auto = item.checked;
          applySettings(saveConfig(next));
          win?.webContents.send('config:changed');
        },
      })),
      { label: L('지금 전부 정리하기', 'Tidy everything now'), click: () => tidyAllNow() },
      { type: 'separator' },
      { label: L('종료', 'Quit'), click: () => app.quit() },
    ]),
  );
}

function createTray() {
  const icon =
    process.platform === 'darwin'
      ? nativeImage.createFromPath(assetPath('trayTemplate.png'))
      : nativeImage.createFromPath(assetPath('tray-color.png'));
  if (process.platform === 'darwin') icon.setTemplateImage(true);
  tray = new Tray(icon);
  if (process.platform !== 'darwin') tray.on('click', () => showWindow());
  updateTray();
}

// ---------- 등짝 스매시 효과 ----------
// 화면 맨 위에 투명 창을 띄워 손바닥이 크게 나타났다가 "찰싹!!" 내리치게 하고, 앱 창도 같이 들썩인다.
// 실제 바탕화면은 움직일 수 없어서, 투명 창 안의 효과와 앱 창 흔들기로 들썩이는 느낌을 낸다.
// smash.css의 타임라인과 맞춘다: 전체 2초, 내리치는 순간 0.8초.
const SMASH_MS = 2000;
const SMASH_HIT_MS = 800;
let smashWin = null;

// 효과를 띄우고 { hit, done } 두 Promise를 돌려준다. hit은 손바닥이 내리치는 순간, done은 효과 창이 닫힌 뒤에 끝난다.
// 효과를 껐으면 둘 다 바로 끝난다. 창이 어떤 이유로 안 뜨거나 안 닫혀도 정해진 시간 안에 반드시 닫는다.
function smashEffect() {
  if (!loadConfig().settings.smash) return { hit: Promise.resolve(), done: Promise.resolve() };
  smashWin?.destroy();
  let hitDone;
  let closeDone;
  const hit = new Promise((r) => (hitDone = r));
  const done = new Promise((r) => (closeDone = r));
  const display = win && win.isVisible() ? screen.getDisplayMatching(win.getBounds()) : screen.getPrimaryDisplay();
  const w = new BrowserWindow({
    ...display.bounds,
    transparent: true,
    frame: false,
    hasShadow: false,
    resizable: false,
    movable: false,
    focusable: false,
    skipTaskbar: true,
    enableLargerThanScreen: true,
    show: false,
    webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true },
  });
  smashWin = w;
  w.setIgnoreMouseEvents(true);
  w.setAlwaysOnTop(true, 'screen-saver');
  w.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true });
  w.webContents.on('will-navigate', (e) => e.preventDefault());
  // 창이 늦게 뜨더라도 정리는 늦어도 1.5초 안에, 창은 늦어도 3초 안에 끝낸다.
  const hitFallback = setTimeout(hitDone, 1500);
  const closeFallback = setTimeout(() => !w.isDestroyed() && w.destroy(), 3000);
  w.once('ready-to-show', () => {
    if (w.isDestroyed()) return;
    w.showInactive();
    setTimeout(() => {
      clearTimeout(hitFallback);
      shakeWindow();
      hitDone();
    }, SMASH_HIT_MS);
    setTimeout(() => !w.isDestroyed() && w.destroy(), SMASH_MS + 100);
  });
  w.on('closed', () => {
    clearTimeout(hitFallback);
    clearTimeout(closeFallback);
    if (smashWin === w) smashWin = null;
    hitDone();
    closeDone();
  });
  w.loadFile(path.join(__dirname, 'renderer', 'smash.html'), { query: { lang: getLang() } });
  return { hit, done };
}

function shakeWindow() {
  if (!win || !win.isVisible() || win.isFullScreen() || win.isMaximized()) return;
  const [x, y] = win.getPosition();
  const steps = [[-16, 10], [14, -9], [-10, -7], [8, 6], [-4, 2], [0, 0]];
  steps.forEach(([dx, dy], i) => setTimeout(() => win?.setPosition(x + dx, y + dy), i * 45));
}

// 파일이 가장 많이 들어간 정리 대상 폴더 안에서, 옮긴 자리들의 공통 상위 폴더를 고른다.
// 예: 문서/사진·이미지와 문서/자료로 옮겼으면 문서, 바탕화면/스크린샷에만 옮겼으면 그 폴더.
function tidiedFolder(cfg, ops) {
  const dirs = ops.filter((o) => o.type === 'move').map((o) => path.dirname(o.to));
  if (!dirs.length) return null;
  const roots = cfg.targets.map((t) => path.resolve(expandHome(t.path))).sort((a, b) => b.length - a.length);
  const groups = new Map();
  for (const d of dirs) {
    const root = roots.find((r) => d === r || d.startsWith(r + path.sep)) || d;
    groups.set(root, [...(groups.get(root) || []), d]);
  }
  const [root, list] = [...groups.entries()].sort((a, b) => b[1].length - a[1].length)[0];
  let common = list[0];
  for (const d of list) while (common !== root && d !== common && !d.startsWith(common + path.sep)) common = path.dirname(common);
  return common;
}

// 정리 버튼·트레이의 정리: 효과를 띄우고, 손바닥이 내리치는 순간 정리한 뒤, 효과 창이 닫힌 다음에 폴더를 연다.
// 폴더가 이미 열려 있으면 Finder가 그 창을 앞으로 가져오는데, 그것도 효과가 다 끝난 뒤에 일어난다.
// 앱의 정리 버튼과 트레이의 정리가 겹치면 앞의 효과를 끊지 않고 차례로 돌린다.
let smashQueue = Promise.resolve();
function smashAndApply(planFor) {
  const run = smashQueue.then(() => smashAndApplyNow(planFor));
  smashQueue = run.catch(() => {});
  return run;
}

async function smashAndApplyNow(planFor) {
  const { hit, done } = smashEffect();
  await hit;
  const cfg = loadConfig();
  const res = planFor(cfg);
  const dir = cfg.settings.openAfter && res.run ? tidiedFolder(cfg, res.run.ops) : null;
  await done;
  if (dir) shell.openPath(dir);
  return res;
}

// ---------- 자동 정리 잔소리 배너 ----------
// 시스템 알림은 서명 안 된 앱에서 안 뜰 때가 있어서, 화면 위쪽 가운데에 투명 창으로 직접 띄운다.
// 누르면 정리한 폴더를 연다. 4.5초 뒤 저절로 사라진다.
const NAG_MS = 4500;
let nagWin = null;

function showNag(sub, folder) {
  nagWin?.destroy();
  const { workArea } = screen.getDisplayNearestPoint(screen.getCursorScreenPoint());
  // 카드(436×70)에 그림자가 잘리지 않을 만큼 여백을 둔 투명 창
  const width = 500;
  const height = 130;
  const w = new BrowserWindow({
    x: Math.round(workArea.x + (workArea.width - width) / 2),
    y: workArea.y,
    width,
    height,
    transparent: true,
    frame: false,
    hasShadow: false,
    resizable: false,
    movable: false,
    focusable: false,
    skipTaskbar: true,
    acceptFirstMouse: true,
    show: false,
    webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true },
  });
  nagWin = w;
  w.setAlwaysOnTop(true, 'screen-saver');
  w.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true });
  w.webContents.on('will-navigate', (e) => e.preventDefault());
  w.webContents.on('did-navigate-in-page', (_e, url) => {
    if (!url.endsWith('#open')) return;
    if (folder) shell.openPath(folder);
    w.destroy();
  });
  w.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  w.once('ready-to-show', () => w.showInactive());
  const timer = setTimeout(() => w.destroy(), NAG_MS + 100);
  w.on('closed', () => {
    clearTimeout(timer);
    if (nagWin === w) nagWin = null;
  });
  w.loadFile(path.join(__dirname, 'renderer', 'nag.html'), { query: { sub, lang: getLang() } });
}

async function tidyAllNow() {
  let total = 0;
  await smashAndApply((cfg) => {
    const ops = [];
    for (const t of cfg.targets) {
      try {
        const { results, run } = applyPlan(cfg, scanTarget(cfg, t.id).items, 'app');
        total += results.filter((r) => r.ok).length;
        if (run) ops.push(...run.ops);
      } catch (err) {
        console.error(err);
      }
    }
    return { run: ops.length ? { ops } : null };
  });
  win?.webContents.send('auto:ran', { count: total });
  if (Notification.isSupported()) new Notification({ title: total ? L(`찰싹!! ${total}개 치웠어요`, `SMACK!! Tidied ${total} files`) : L('치울 게 없네요. 깨끗해요!', 'Nothing to tidy. All clean!'), silent: true }).show();
}

// 개발용: 화면을 PNG로 저장하고 종료한다. SMASH_CAPTURE=저장경로 [SMASH_CAPTURE_JS=실행할 스크립트]
function setupCapture(w) {
  if (app.isPackaged || !process.env.SMASH_CAPTURE) return;
  w.webContents.once('did-finish-load', async () => {
    await new Promise((r) => setTimeout(r, 1500));
    if (process.env.SMASH_CAPTURE_JS) {
      const out = await w.webContents.executeJavaScript(process.env.SMASH_CAPTURE_JS);
      if (out !== undefined) fs.writeFileSync(`${process.env.SMASH_CAPTURE}.json`, JSON.stringify(out, null, 2));
      await new Promise((r) => setTimeout(r, Number(process.env.SMASH_CAPTURE_WAIT) || 800));
    }
    const img = await w.webContents.capturePage();
    fs.writeFileSync(process.env.SMASH_CAPTURE, img.toPNG());
    quitting = true;
    app.quit();
  });
}

// ---------- IPC ----------

function handle(channel, fn) {
  ipcMain.handle(channel, async (_e, ...args) => {
    try {
      return { ok: true, data: await fn(...args) };
    } catch (err) {
      return { ok: false, error: err.message };
    }
  });
}

handle('config:get', () => ({
  config: loadConfig(),
  configPath: label(configPath()),
  home: os.homedir(),
  sep: path.sep,
  categories: localCategories(),
  lang: getLang(),
  platform: process.platform,
}));
handle('config:save', (cfg) => {
  const saved = saveConfig(cfg);
  applySettings(saved);
  return saved;
});

handle('scan', (targetId) => {
  const res = scanTarget(loadConfig(), targetId);
  return { ...res, rootLabel: label(res.root), items: decorate(res.items), unmatched: res.unmatched.map((f) => ({ ...f, srcLabel: label(f.path) })) };
});
handle('folders', () => folderTree(loadConfig(), 3).map(label));
handle('rule:test', (targetId, rule) => testRule(loadConfig(), targetId, rule));
handle('rule:fromText', (targetId, text) => {
  const cfg = loadConfig();
  const target = findTarget(cfg, targetId);
  if (!target) throw new Error(L('폴더를 찾을 수 없습니다', 'Folder not found'));
  return ruleFromText(cfg, path.resolve(expandHome(target.path)), String(text).slice(0, 500));
});

handle('apply', (items) => smashAndApply((cfg) => applyPlan(cfg, items, 'app')));
handle('undo', () => undoLast());
handle('history', () =>
  loadHistory()
    .reverse()
    .map((r) => ({ ...r, ops: r.ops.map((o) => ({ ...o, fromLabel: label(o.from), toLabel: label(o.to) })) })),
);

handle('ai:suggest', async (targetId, unmatched) => {
  const cfg = loadConfig();
  const target = findTarget(cfg, targetId);
  if (!target) throw new Error(L('폴더를 찾을 수 없습니다', 'Folder not found'));
  const res = await suggestPlacements(cfg, path.resolve(expandHome(target.path)), unmatched);
  return { ...res, items: decorate(res.items) };
});

handle('integration:status', async () => ({ ...(await integrationStatus()), launch: mcpLaunch(), mcpName: MCP_NAME }));
handle('integration:claudeCode', (locationIds) => registerClaudeCode(mcpLaunch(), locationIds));
handle('integration:claudeDesktop', () => registerClaudeDesktop(mcpLaunch()));

handle('dialog:folder', async (defaultPath) => {
  const res = await dialog.showOpenDialog(win, { properties: ['openDirectory', 'createDirectory'], defaultPath });
  return res.canceled ? null : res.filePaths[0];
});
handle('shell:show', (p) => shell.showItemInFolder(p));
handle('shell:openUrl', (url) => {
  if (/^https:\/\//.test(url)) shell.openExternal(url);
});

// ---------- 시작 ----------

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on('second-instance', () => showWindow());
  app.whenReady().then(() => {
    // 처음 실행이면 기본 설정(폴더·규칙 이름)도 이 언어로 만들어진다.
    syncLang({ settings: {} });
    syncLang(loadConfig());
    const startHidden = process.argv.includes('--hidden') || app.getLoginItemSettings().wasOpenedAsHidden;
    createTray();
    createWindow({ show: !startHidden });
    if (startHidden && process.platform === 'darwin') app.dock?.hide();
    applySettings(loadConfig());
    app.on('activate', () => showWindow());
  });
  app.on('before-quit', () => {
    quitting = true;
    auto.stop();
  });
  app.on('window-all-closed', () => {
    if (!anyAuto()) app.quit();
  });
}
