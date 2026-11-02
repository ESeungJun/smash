'use strict';
// 사용자 PC에 로그인된 Claude와 연동한다. 앱은 API 키를 갖지 않는다.
//  - claude CLI(`claude -p`)로 분류 제안 받기
//  - 이 앱의 MCP 서버를 Claude Code / Claude Desktop에 등록하기
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawn, execFileSync } = require('child_process');
const { contractHome, expandHome, writeJsonAtomic } = require('./config');
const { folderTree, planFromMoves } = require('./organizer');

const MCP_NAME = 'smash';
// 예전 이름으로 등록된 서버는 다시 등록할 때 지운다.
const LEGACY_MCP_NAMES = ['folder-tidy', 'deungjjak'];
const MAX_FILES_PER_REQUEST = 200;
const SUGGEST_TIMEOUT_MS = 180_000;

const SUGGESTION_SCHEMA = {
  type: 'object',
  properties: {
    suggestions: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          file: { type: 'string' },
          dest: { type: ['string', 'null'] },
          reason: { type: 'string' },
        },
        required: ['file', 'dest', 'reason'],
      },
    },
  },
  required: ['suggestions'],
};

function candidatePaths() {
  const home = os.homedir();
  if (process.platform === 'win32') {
    const appData = process.env.APPDATA || path.join(home, 'AppData', 'Roaming');
    const local = process.env.LOCALAPPDATA || path.join(home, 'AppData', 'Local');
    return [
      path.join(home, '.local', 'bin', 'claude.exe'),
      path.join(local, 'Programs', 'claude', 'claude.exe'),
      path.join(appData, 'npm', 'claude.cmd'),
    ];
  }
  return [
    path.join(home, '.local', 'bin', 'claude'),
    path.join(home, '.claude', 'local', 'claude'),
    '/opt/homebrew/bin/claude',
    '/usr/local/bin/claude',
  ];
}

let cachedCli;
function findClaudeCli() {
  if (cachedCli && fs.existsSync(cachedCli)) return cachedCli;
  cachedCli = candidatePaths().find((p) => fs.existsSync(p)) || null;
  if (!cachedCli) {
    // GUI 앱은 셸 PATH를 모르기 때문에 로그인 셸에 한 번 물어본다.
    try {
      const out =
        process.platform === 'win32'
          ? execFileSync('where.exe', ['claude'], { encoding: 'utf8', windowsHide: true })
          : execFileSync(process.env.SHELL || '/bin/zsh', ['-lc', 'command -v claude'], { encoding: 'utf8', timeout: 5000 });
      // 셸 프로필이 인사말 등을 찍을 수 있어 실제로 있는 경로만 고른다.
      const found = out.split(/\r?\n/).map((s) => s.trim()).find((s) => path.isAbsolute(s) && fs.existsSync(s));
      if (found) cachedCli = found;
    } catch {
      /* 설치 안 됨 */
    }
  }
  return cachedCli;
}

function runCli(args, { input, timeoutMs = 30_000, configDir } = {}) {
  const cli = findClaudeCli();
  if (!cli) return Promise.reject(new Error('Claude Code CLI를 찾지 못했습니다. claude.com/claude-code 에서 설치 후 로그인해 주세요.'));
  // npm으로 설치한 Windows의 claude.cmd는 셸을 거쳐야 실행된다.
  const viaShell = /\.cmd$/i.test(cli);
  const quote = (a) => (a === '' || /[\s"&|<>^%]/.test(a) ? `"${a.replace(/"/g, '""')}"` : a);
  const finalArgs = viaShell ? args.map(quote) : args;
  return new Promise((resolve, reject) => {
    // configDir: undefined면 현재 환경 그대로, null이면 기본 위치(~/.claude), 문자열이면 그 위치.
    const env = { ...process.env };
    if (configDir === null) delete env.CLAUDE_CONFIG_DIR;
    else if (configDir) env.CLAUDE_CONFIG_DIR = configDir;
    const child = spawn(viaShell ? quote(cli) : cli, finalArgs, { cwd: os.tmpdir(), shell: viaShell, windowsHide: true, env });
    let stdout = '';
    let stderr = '';
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error('Claude 응답 시간이 초과됐습니다'));
    }, timeoutMs);
    child.stdout.on('data', (d) => (stdout += d));
    child.stderr.on('data', (d) => (stderr += d));
    child.on('error', (err) => {
      clearTimeout(timer);
      reject(err);
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      if (code === 0) resolve(stdout);
      else reject(new Error((stderr || stdout).trim().slice(0, 500) || `claude 종료 코드 ${code}`));
    });
    // 입력이 없어도 stdin을 닫아야 CLI가 입력을 기다리지 않는다.
    child.stdin.end(input ?? undefined);
  });
}

async function cliStatus() {
  const cli = findClaudeCli();
  if (!cli) return { installed: false };
  try {
    const version = (await runCli(['--version'], { timeoutMs: 10_000 })).trim();
    return { installed: true, path: contractHome(cli), version };
  } catch (err) {
    return { installed: true, path: contractHome(cli), error: err.message };
  }
}

function buildPrompt(targetRoot, files, folders) {
  const fmtDate = (ms) => new Date(ms).toISOString().slice(0, 10);
  const fmtSize = (n) => (n >= 1 << 20 ? `${(n / (1 << 20)).toFixed(1)}MB` : `${Math.max(1, Math.round(n / 1024))}KB`);
  return [
    '너는 파일 정리 도우미야. 아래 파일 각각을 가장 알맞은 폴더로 분류해 줘.',
    '규칙:',
    '- dest에는 "폴더 목록"에 있는 경로를 그대로 쓴다. 꼭 필요할 때만 정리 대상 폴더 바로 아래 새 폴더를 만든다(예: ~/Downloads/새폴더).',
    '- 어디가 맞는지 확신이 없으면 dest를 null로 둔다.',
    '- 삭제는 제안하지 않는다.',
    '- reason은 한국어로 30자 이내.',
    '- file에는 파일 이름을 그대로 쓴다.',
    '',
    `지금 정리하는 폴더: ${contractHome(targetRoot)}`,
    '',
    '폴더 목록:',
    ...folders.map((f) => `- ${contractHome(f)}`),
    '',
    '파일 목록 (이름 | 크기 | 수정일):',
    ...files.map((f) => `- ${f.name} | ${fmtSize(f.size)} | ${fmtDate(f.mtimeMs)}`),
    '',
    '응답은 {"suggestions":[{"file":"...","dest":"..." 또는 null,"reason":"..."}]} 형식의 JSON만.',
  ].join('\n');
}

function parseSuggestions(stdout) {
  const outer = JSON.parse(stdout);
  if (outer.is_error) throw new Error(String(outer.result || 'Claude 오류'));
  if (outer.structured_output?.suggestions) return outer.structured_output.suggestions;
  const text = String(outer.result || '');
  const start = text.indexOf('{');
  const end = text.lastIndexOf('}');
  if (start < 0 || end < start) throw new Error('Claude 응답을 해석하지 못했습니다');
  return JSON.parse(text.slice(start, end + 1)).suggestions || [];
}

async function suggestPlacements(cfg, targetRoot, unmatched) {
  const files = unmatched.slice(0, MAX_FILES_PER_REQUEST);
  if (!files.length) return { items: [], rejected: [], noSuggestion: [] };
  const prompt = buildPrompt(targetRoot, files, folderTree(cfg));
  const args = ['-p', '--output-format', 'json', '--tools', '', '--no-session-persistence', '--strict-mcp-config', '--setting-sources', ''];
  if (cfg.ai?.model) args.push('--model', cfg.ai.model);
  if (!/\.cmd$/i.test(findClaudeCli() || '')) args.push('--json-schema', JSON.stringify(SUGGESTION_SCHEMA));
  const stdout = await runCli(args, { input: prompt, timeoutMs: SUGGEST_TIMEOUT_MS });
  const byName = new Map(files.map((f) => [f.name, f]));
  const moves = [];
  const noSuggestion = [];
  for (const s of parseSuggestions(stdout)) {
    const file = byName.get(s.file);
    if (!file) continue;
    byName.delete(s.file);
    if (!s.dest) noSuggestion.push(file.name);
    else moves.push({ src: file.path, destFolder: expandHome(s.dest), reason: `AI: ${s.reason || ''}`.trim() });
  }
  noSuggestion.push(...byName.keys());
  const plan = planFromMoves(cfg, moves);
  return { ...plan, noSuggestion };
}

// ---- 말로 규칙 만들기 ----

const RULE_SCHEMA = {
  type: 'object',
  properties: {
    name: { type: 'string' },
    extensions: { type: 'array', items: { type: 'string' } },
    nameIncludes: { type: 'array', items: { type: 'string' } },
    olderThanDays: { type: 'number' },
    action: { type: 'string', enum: ['move', 'trash'] },
    dest: { type: 'string' },
  },
  required: ['name', 'extensions', 'nameIncludes', 'olderThanDays', 'action', 'dest'],
};

async function ruleFromText(cfg, targetRoot, text) {
  const folders = folderTree(cfg).map(contractHome);
  const prompt = [
    '사용자가 말로 설명한 파일 정리 규칙을 JSON으로 바꿔 줘.',
    `정리 대상 폴더: ${contractHome(targetRoot)}`,
    '- extensions: 점 없는 소문자 확장자 목록. 종류만 말하면 흔한 확장자를 채운다(예: 사진 → jpg, jpeg, png, heic, gif, webp).',
    '- nameIncludes: 파일 이름에 들어 있어야 하는 단어. 없으면 빈 배열.',
    '- olderThanDays: "일주일 지난" 같은 조건이 있으면 일 수, 없으면 0.',
    '- action: 옮기면 move, 지우라고 하면 trash(실제로는 휴지통으로 간다).',
    '- dest: move일 때 목적지. 아래 폴더 목록에 맞는 게 있으면 그 경로(~ 경로)를 그대로 쓰고, 없으면 정리 대상 폴더 기준 상대 경로로 새 폴더 이름을 쓴다. trash면 빈 문자열.',
    '- name: 한국어 규칙 이름, 15자 이내.',
    '',
    '폴더 목록:',
    ...folders.map((f) => `- ${f}`),
    '',
    `사용자 설명: ${text}`,
  ].join('\n');
  const args = ['-p', '--output-format', 'json', '--tools', '', '--no-session-persistence', '--strict-mcp-config', '--setting-sources', ''];
  if (cfg.ai?.model) args.push('--model', cfg.ai.model);
  if (!/\.cmd$/i.test(findClaudeCli() || '')) args.push('--json-schema', JSON.stringify(RULE_SCHEMA));
  const outer = JSON.parse(await runCli(args, { input: prompt, timeoutMs: SUGGEST_TIMEOUT_MS }));
  if (outer.is_error) throw new Error(String(outer.result || 'Claude 오류'));
  let rule = outer.structured_output;
  if (!rule) {
    const t = String(outer.result || '');
    rule = JSON.parse(t.slice(t.indexOf('{'), t.lastIndexOf('}') + 1));
  }
  return {
    name: String(rule.name || '새 규칙'),
    match: {
      extensions: (rule.extensions || []).map((e) => String(e).toLowerCase().replace(/^\./, '')).filter(Boolean),
      nameIncludes: (rule.nameIncludes || []).map(String).filter(Boolean),
      olderThanDays: Math.max(0, Number(rule.olderThanDays) || 0),
    },
    action: rule.action === 'trash' ? 'trash' : 'move',
    dest: rule.action === 'trash' ? '' : String(rule.dest || ''),
  };
}

// ---- MCP 등록 ----

// Claude Code 설정 위치들. 기본(~/.claude.json) + CLAUDE_CONFIG_DIR + Orca 앱 프로필.
function claudeConfigLocations() {
  const home = os.homedir();
  const locations = [{ id: 'default', label: '기본 (~/.claude.json)', dir: null, file: path.join(home, '.claude.json') }];
  const add = (dir, label) => {
    const resolved = path.resolve(dir);
    if (locations.some((l) => l.dir === resolved)) return;
    locations.push({ id: resolved, label, dir: resolved, file: path.join(resolved, '.claude.json') });
  };
  const orca = path.join(home, 'Library', 'Application Support', 'orca', 'claude-profiles');
  let selected = '';
  try {
    selected = fs.readFileSync(path.join(orca, 'selected-host'), 'utf8').trim();
  } catch {
    /* Orca 미사용 */
  }
  let profiles = [];
  try {
    profiles = fs.readdirSync(orca, { withFileTypes: true }).filter((d) => d.isDirectory());
  } catch {
    /* Orca 미사용 */
  }
  for (const d of profiles) {
    const dir = path.join(orca, d.name, 'home');
    if (!fs.existsSync(path.join(dir, '.claude.json'))) continue;
    add(dir, `Orca 프로필 ${d.name.slice(0, 8)}${path.resolve(selected) === path.resolve(dir) ? ' (사용 중)' : ''}`);
  }
  if (process.env.CLAUDE_CONFIG_DIR) add(process.env.CLAUDE_CONFIG_DIR, `CLAUDE_CONFIG_DIR (${contractHome(process.env.CLAUDE_CONFIG_DIR)})`);
  return locations;
}

function registeredIn(location) {
  try {
    const data = JSON.parse(fs.readFileSync(location.file, 'utf8'));
    return Boolean(data.mcpServers?.[MCP_NAME]);
  } catch {
    return false;
  }
}

async function registerClaudeCode(launch, locationIds) {
  const envArgs = Object.entries(launch.env || {}).flatMap(([k, v]) => ['-e', `${k}=${v}`]);
  const targets = claudeConfigLocations().filter((l) => !locationIds || locationIds.includes(l.id));
  const results = [];
  for (const loc of targets) {
    try {
      for (const name of [MCP_NAME, ...LEGACY_MCP_NAMES]) {
        await runCli(['mcp', 'remove', '-s', 'user', name], { configDir: loc.dir }).catch(() => {});
      }
      // -e는 값을 여러 개 받는 옵션이라 서버 이름보다 뒤에 둬야 이름을 삼키지 않는다.
      await runCli(['mcp', 'add', '-s', 'user', MCP_NAME, ...envArgs, '--', launch.command, ...launch.args], { configDir: loc.dir });
      results.push({ location: loc.label, ok: registeredIn(loc) });
    } catch (err) {
      results.push({ location: loc.label, ok: false, error: err.message });
    }
  }
  return results;
}

function claudeDesktopConfigPath() {
  const home = os.homedir();
  if (process.platform === 'darwin') return path.join(home, 'Library', 'Application Support', 'Claude', 'claude_desktop_config.json');
  if (process.platform === 'win32') return path.join(process.env.APPDATA || path.join(home, 'AppData', 'Roaming'), 'Claude', 'claude_desktop_config.json');
  return path.join(process.env.XDG_CONFIG_HOME || path.join(home, '.config'), 'Claude', 'claude_desktop_config.json');
}

function readDesktopConfig() {
  const file = claudeDesktopConfigPath();
  if (!fs.existsSync(file)) return { file, data: {} };
  return { file, data: JSON.parse(fs.readFileSync(file, 'utf8')) };
}

function registerClaudeDesktop(launch) {
  const { file, data } = readDesktopConfig();
  if (fs.existsSync(file)) fs.copyFileSync(file, `${file}.bak`);
  const servers = { ...(data.mcpServers || {}) };
  for (const name of LEGACY_MCP_NAMES) delete servers[name];
  servers[MCP_NAME] = launch;
  data.mcpServers = servers;
  writeJsonAtomic(file, data);
  return contractHome(file);
}

async function integrationStatus() {
  const cli = await cliStatus();
  const locations = claudeConfigLocations().map((l) => ({ id: l.id, label: l.label, registered: registeredIn(l) }));
  let claudeDesktop = false;
  let desktopInstalled = false;
  try {
    const { file, data } = readDesktopConfig();
    desktopInstalled = fs.existsSync(path.dirname(file));
    claudeDesktop = Boolean(data.mcpServers?.[MCP_NAME]);
  } catch {
    /* 설정 파일이 깨져 있으면 등록 안 된 것으로 본다 */
  }
  return {
    cli,
    locations,
    claudeCode: locations.some((l) => l.registered),
    claudeDesktop,
    desktopInstalled,
    desktopConfig: contractHome(claudeDesktopConfigPath()),
  };
}

module.exports = {
  MCP_NAME,
  findClaudeCli,
  cliStatus,
  suggestPlacements,
  buildPrompt,
  parseSuggestions,
  registerClaudeCode,
  registerClaudeDesktop,
  integrationStatus,
  claudeConfigLocations,
  ruleFromText,
};
