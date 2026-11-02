'use strict';
// 폴더 스캔 → 정리 계획 → 실행 → 기록·되돌리기.
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { expandHome, contractHome, historyPath, writeJsonAtomic, newId } = require('./config');
const { moveToTrash } = require('./trash');

const DAY_MS = 24 * 60 * 60 * 1000;
const HISTORY_LIMIT = 50;
// 폴더 목록을 만들 때 들어가지 않는 폴더(내부 구조라 정리 대상이 아님)
const OPAQUE_DIRS = new Set(['node_modules', '.git', 'Unreal Projects']);

function globToRegex(glob) {
  const body = glob.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.');
  return new RegExp(`^${body}$`, 'i');
}

function makeSkipper(cfg) {
  const patterns = (cfg.skip?.patterns || []).map((p) => globToRegex(nfc(p)));
  return (name) => (cfg.skip?.hidden !== false && name.startsWith('.')) || patterns.some((re) => re.test(nfc(name)));
}

function extOf(name) {
  const ext = path.extname(name);
  return ext ? ext.slice(1).toLowerCase() : '';
}

function isInside(child, parent) {
  const rel = path.relative(parent, child);
  return rel !== '' && !rel.startsWith('..') && !path.isAbsolute(rel);
}

function targetRoots(cfg) {
  return cfg.targets.map((t) => path.resolve(expandHome(t.path))).filter(Boolean);
}

function insideAnyRoot(cfg, p) {
  const abs = path.resolve(p);
  return targetRoots(cfg).some((root) => abs === root || isInside(abs, root));
}

function findTarget(cfg, ref) {
  if (!ref) return cfg.targets[0];
  const abs = path.resolve(expandHome(ref));
  return cfg.targets.find((t) => t.id === ref || path.resolve(expandHome(t.path)) === abs);
}

function ruleHasCondition(rule) {
  const m = rule.match || {};
  return (m.extensions?.length || 0) + (m.nameIncludes?.length || 0) > 0 || m.olderThanDays > 0;
}

// macOS는 파일 이름의 한글을 자모로 풀어서(NFD) 돌려준다. 사용자가 입력한 단어(NFC)와 비교하려면 맞춰야 한다.
const nfc = (s) => String(s).normalize('NFC');

function matchRule(rule, file, now = Date.now()) {
  if (!rule.enabled || !ruleHasCondition(rule)) return false;
  const m = rule.match;
  if (m.extensions.length && !m.extensions.includes(extOf(file.name))) return false;
  if (m.nameIncludes.length) {
    const lower = nfc(file.name).toLowerCase();
    if (!m.nameIncludes.some((s) => lower.includes(nfc(s).toLowerCase()))) return false;
  }
  if (m.olderThanDays > 0 && now - file.mtimeMs < m.olderThanDays * DAY_MS) return false;
  return true;
}

function resolveDestDir(targetRoot, dest) {
  const expanded = expandHome(String(dest || '').trim());
  return path.isAbsolute(expanded) ? path.resolve(expanded) : path.resolve(targetRoot, expanded);
}

function hashFile(file) {
  const hash = crypto.createHash('sha256');
  const fd = fs.openSync(file, 'r');
  try {
    const buf = Buffer.allocUnsafe(1 << 20);
    let n;
    while ((n = fs.readSync(fd, buf, 0, buf.length, null)) > 0) hash.update(buf.subarray(0, n));
  } finally {
    fs.closeSync(fd);
  }
  return hash.digest('hex');
}

function sameContent(a, b) {
  const sa = fs.statSync(a);
  const sb = fs.statSync(b);
  return sa.size === sb.size && hashFile(a) === hashFile(b);
}

function uniqueName(dir, name, reserved) {
  const ext = path.extname(name);
  const stem = path.basename(name, ext);
  for (let i = 2; ; i += 1) {
    const candidate = path.join(dir, `${stem} (${i})${ext}`);
    if (!fs.existsSync(candidate) && !reserved.has(candidate)) return candidate;
  }
}

// 같은 이름이 이미 있으면: 내용까지 같으면 중복으로 휴지통, 다르면 " (2)"를 붙인다.
function planMove(src, destDir, reserved) {
  const name = path.basename(src);
  const dest = path.join(destDir, name);
  if (path.resolve(path.dirname(src)) === path.resolve(destDir)) return null;
  if (reserved.has(dest)) {
    const renamed = uniqueName(destDir, name, reserved);
    reserved.add(renamed);
    return { action: 'move', dest: renamed, note: 'renamed' };
  }
  if (fs.existsSync(dest)) {
    if (fs.statSync(dest).isFile() && sameContent(src, dest)) return { action: 'trash', dest: '', note: 'duplicate', duplicateOf: dest };
    const renamed = uniqueName(destDir, name, reserved);
    reserved.add(renamed);
    return { action: 'move', dest: renamed, note: 'renamed' };
  }
  reserved.add(dest);
  return { action: 'move', dest, note: null };
}

function listFiles(dir, skip) {
  return fs
    .readdirSync(dir, { withFileTypes: true })
    .filter((d) => d.isFile() && !skip(d.name))
    .map((d) => {
      const full = path.join(dir, d.name);
      const st = fs.statSync(full);
      return { name: d.name, path: full, size: st.size, mtimeMs: st.mtimeMs };
    })
    .sort((a, b) => a.name.localeCompare(b.name, 'ko'));
}

function makeItem(file, planned, extra) {
  return {
    id: newId('i'),
    src: file.path,
    name: file.name,
    size: file.size,
    action: planned.action,
    dest: planned.dest,
    note: planned.note,
    duplicateOf: planned.duplicateOf || null,
    ...extra,
  };
}

function scanTarget(cfg, ref) {
  const target = findTarget(cfg, ref);
  if (!target) throw new Error(`설정에 없는 폴더입니다: ${ref}`);
  const root = path.resolve(expandHome(target.path));
  if (!fs.existsSync(root)) throw new Error(`폴더가 없습니다: ${target.path}`);
  const skip = makeSkipper(cfg);
  const reserved = new Set();
  const now = Date.now();
  const items = [];
  const unmatched = [];
  for (const file of listFiles(root, skip)) {
    const rule = target.rules.find((r) => matchRule(r, file, now));
    if (!rule) {
      unmatched.push({ name: file.name, path: file.path, size: file.size, mtimeMs: file.mtimeMs });
      continue;
    }
    const planned =
      rule.action === 'trash' ? { action: 'trash', dest: '', note: null } : planMove(file.path, resolveDestDir(root, rule.dest), reserved);
    if (planned) items.push(makeItem(file, planned, { ruleId: rule.id, reason: `규칙: ${rule.name}` }));
  }
  return { targetId: target.id, root, items, unmatched };
}

// 규칙 편집 중 미리보기: 이 규칙 하나에 지금 맞는 파일들.
function testRule(cfg, ref, rule) {
  const target = findTarget(cfg, ref);
  if (!target) throw new Error(`설정에 없는 폴더입니다: ${ref}`);
  const root = path.resolve(expandHome(target.path));
  const candidate = { enabled: true, ...rule, match: { extensions: [], nameIncludes: [], olderThanDays: 0, ...(rule.match || {}) } };
  const now = Date.now();
  return listFiles(root, makeSkipper(cfg))
    .filter((f) => matchRule(candidate, f, now))
    .map((f) => f.name);
}

// 아직 없는 목적지 폴더들(미리보기에서 '새 폴더' 표시용).
function missingDirs(dest) {
  const out = [];
  for (let dir = path.dirname(dest); !fs.existsSync(dir); dir = path.dirname(dir)) {
    out.push(dir);
    if (path.dirname(dir) === dir) break;
  }
  return out;
}

// AI·MCP가 만든 이동 목록을 검증해서 실행 가능한 계획으로 바꾼다.
function planFromMoves(cfg, moves) {
  const reserved = new Set();
  const items = [];
  const rejected = [];
  for (const m of moves) {
    const src = path.resolve(expandHome(String(m.src || '')));
    const fail = (why) => rejected.push({ src: m.src, reason: why });
    if (!insideAnyRoot(cfg, src)) { fail('설정한 정리 대상 폴더 밖의 파일입니다'); continue; }
    let st;
    try { st = fs.lstatSync(src); } catch { fail('파일이 없습니다'); continue; }
    if (!st.isFile()) { fail('파일만 옮길 수 있습니다'); continue; }
    const file = { name: path.basename(src), path: src, size: st.size, mtimeMs: st.mtimeMs };
    const reason = String(m.reason || '');
    if (m.action === 'trash') {
      items.push(makeItem(file, { action: 'trash', dest: '', note: null }, { ruleId: null, reason }));
      continue;
    }
    const destDir = resolveDestDir(path.dirname(src), m.destFolder ?? m.dest_folder ?? m.dest);
    if (!insideAnyRoot(cfg, destDir)) { fail('목적지가 정리 대상 폴더 밖입니다'); continue; }
    const planned = planMove(src, destDir, reserved);
    if (!planned) { fail('이미 그 폴더에 있습니다'); continue; }
    items.push(makeItem(file, planned, { ruleId: null, reason }));
  }
  return { items, rejected };
}

function moveFile(from, to) {
  fs.mkdirSync(path.dirname(to), { recursive: true });
  try {
    fs.renameSync(from, to);
  } catch (err) {
    if (err.code !== 'EXDEV') throw err;
    const st = fs.statSync(from);
    fs.copyFileSync(from, to, fs.constants.COPYFILE_EXCL);
    fs.utimesSync(to, st.atime, st.mtime);
    fs.unlinkSync(from);
  }
}

function loadHistory() {
  try {
    return JSON.parse(fs.readFileSync(historyPath(), 'utf8'));
  } catch {
    return [];
  }
}

function saveHistory(runs) {
  writeJsonAtomic(historyPath(), runs.slice(-HISTORY_LIMIT));
}

function applyPlan(cfg, items, source = 'app') {
  const ops = [];
  const results = [];
  const reserved = new Set();
  for (let item of items) {
    const src = path.resolve(String(item.src || ''));
    const done = (ok, message) => results.push({ id: item.id, name: path.basename(src), ok, message });
    try {
      if (!insideAnyRoot(cfg, src)) throw new Error('정리 대상 폴더 밖의 파일입니다');
      if (!fs.lstatSync(src).isFile()) throw new Error('파일이 아닙니다');
      // 중복이라 버리기로 한 파일은 원본이 아직 그대로 있을 때만 버린다. 아니면 원본 자리로 옮긴다.
      if (item.action === 'trash' && item.duplicateOf) {
        const original = path.resolve(String(item.duplicateOf));
        const stillDuplicate = fs.existsSync(original) && fs.statSync(original).isFile() && sameContent(src, original);
        if (!stillDuplicate) item = { ...item, action: 'move', dest: original };
      }
      if (item.action === 'trash') {
        moveToTrash(src);
        ops.push({ type: 'trash', from: src });
        done(true, '휴지통으로 보냄');
        continue;
      }
      const wanted = path.resolve(String(item.dest || ''));
      if (!insideAnyRoot(cfg, wanted)) throw new Error('목적지가 정리 대상 폴더 밖입니다');
      // 계획 이후에 상황이 바뀌었을 수 있어 실행 직전에 다시 확인한다.
      const planned = planMove(src, path.dirname(wanted), reserved);
      if (!planned) throw new Error('이미 그 폴더에 있습니다');
      if (planned.action === 'trash') {
        moveToTrash(src);
        ops.push({ type: 'trash', from: src });
        done(true, '같은 파일이 이미 있어 휴지통으로 보냄');
        continue;
      }
      const to = planned.dest;
      moveFile(src, to);
      ops.push({ type: 'move', from: src, to });
      done(true, `→ ${contractHome(to)}`);
    } catch (err) {
      done(false, err.message);
    }
  }
  let run = null;
  if (ops.length) {
    run = { id: newId('run'), at: new Date().toISOString(), source, ops, undone: false };
    saveHistory([...loadHistory(), run]);
  }
  return { results, run };
}

function undoLast() {
  const runs = loadHistory();
  const run = [...runs].reverse().find((r) => !r.undone);
  if (!run) return { ok: false, message: '되돌릴 작업이 없습니다', restored: 0, failed: [], trashed: [] };
  let restored = 0;
  const failed = [];
  for (const op of [...run.ops].reverse()) {
    if (op.type !== 'move') continue;
    try {
      if (!fs.existsSync(op.to)) throw new Error('옮긴 파일이 없습니다');
      if (fs.existsSync(op.from)) throw new Error('원래 자리에 같은 이름 파일이 있습니다');
      moveFile(op.to, op.from);
      restored += 1;
    } catch (err) {
      failed.push({ file: contractHome(op.to), reason: err.message });
    }
  }
  run.undone = true;
  saveHistory(runs);
  const trashed = run.ops.filter((o) => o.type === 'trash').map((o) => contractHome(o.from));
  return { ok: true, runId: run.id, restored, failed, trashed };
}

// AI가 고를 수 있는 폴더 목록(정리 대상 폴더와 그 하위 폴더 2단계).
function folderTree(cfg, depth = 2) {
  const skip = makeSkipper(cfg);
  const out = [];
  const walk = (dir, level) => {
    out.push(dir);
    if (level >= depth || OPAQUE_DIRS.has(path.basename(dir)) || fs.existsSync(path.join(dir, '.git'))) return;
    let entries = [];
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
    for (const d of entries) {
      if (!d.isDirectory() || skip(d.name) || d.name.endsWith('.app') || d.name.endsWith('_files')) continue;
      walk(path.join(dir, d.name), level + 1);
    }
  };
  for (const root of targetRoots(cfg)) if (fs.existsSync(root)) walk(root, 0);
  return out;
}

module.exports = {
  matchRule,
  testRule,
  missingDirs,
  targetRoots,
  scanTarget,
  planFromMoves,
  applyPlan,
  undoLast,
  loadHistory,
  folderTree,
  findTarget,
  insideAnyRoot,
  extOf,
};
