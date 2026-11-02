'use strict';
const test = require('node:test');
const assert = require('node:assert');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { spawn } = require('child_process');

// 실제 설정·폴더를 건드리지 않도록 전부 임시 폴더에서 돈다.
const sandbox = fs.mkdtempSync(path.join(os.tmpdir(), 'folder-tidy-test-'));
process.env.SMASH_HOME = path.join(sandbox, 'config');

const { saveConfig, loadConfig } = require('../src/core/config');
const { matchRule, scanTarget, applyPlan, undoLast, planFromMoves, loadHistory, testRule, missingDirs } = require('../src/core/organizer');
const { AutoTidy } = require('../src/core/watcher');
const { parseSuggestions, buildPrompt } = require('../src/core/claude');

const inbox = path.join(sandbox, 'inbox');
const archive = path.join(sandbox, 'archive');

function write(p, content, ageDays = 0) {
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
  if (ageDays) {
    const t = new Date(Date.now() - ageDays * 864e5);
    fs.utimesSync(p, t, t);
  }
}

function setup() {
  fs.rmSync(inbox, { recursive: true, force: true });
  fs.rmSync(archive, { recursive: true, force: true });
  fs.rmSync(process.env.SMASH_HOME, { recursive: true, force: true });
  write(path.join(inbox, 'photo.JPG'), 'img');
  write(path.join(inbox, 'report.pdf'), 'new report');
  write(path.join(inbox, 'same.pdf'), 'identical');
  write(path.join(inbox, '연봉 계약서.pdf'), 'contract');
  write(path.join(inbox, 'old-installer.dmg'), 'dmg', 30);
  write(path.join(inbox, 'new-installer.dmg'), 'dmg2');
  write(path.join(inbox, 'mystery.bin'), '???');
  write(path.join(inbox, '.hidden'), 'x');
  write(path.join(inbox, 'video.crdownload'), 'partial');
  write(path.join(inbox, '문서', 'report.pdf'), 'old report');
  write(path.join(inbox, '문서', 'same.pdf'), 'identical');
  fs.mkdirSync(archive, { recursive: true });
  return saveConfig({
    targets: [
      {
        id: 'inbox',
        path: inbox,
        rules: [
          { name: '계약서', match: { nameIncludes: ['계약서'] }, action: 'move', dest: archive },
          { name: '오래된 설치 파일', match: { extensions: ['dmg'], olderThanDays: 7 }, action: 'trash' },
          { name: '이미지', match: { extensions: ['jpg'] }, action: 'move', dest: '이미지' },
          { name: '문서', match: { extensions: ['pdf'] }, action: 'move', dest: '문서' },
          { name: '조건 없는 규칙', match: {}, action: 'move', dest: '어디든' },
        ],
      },
      { id: 'archive', path: archive, rules: [] },
    ],
  });
}

test('matchRule: 조건 없는 규칙은 아무것도 잡지 않는다', () => {
  const rule = { enabled: true, match: { extensions: [], nameIncludes: [], olderThanDays: 0 } };
  assert.equal(matchRule(rule, { name: 'a.txt', mtimeMs: 0 }), false);
});

test('matchRule: macOS가 자모로 풀어 준(NFD) 한글 파일 이름도 단어로 잡는다', () => {
  const rule = { enabled: true, match: { extensions: ['png'], nameIncludes: ['스크린샷'], olderThanDays: 0 } };
  const name = '스크린샷 2026-11-01 오후 9.20.11.png'.normalize('NFD');
  assert.equal(matchRule(rule, { name, mtimeMs: 0 }), true);
});

test('scan: 규칙 순서·중복·이름 충돌·건너뛰기', () => {
  const cfg = setup();
  const { items, unmatched } = scanTarget(cfg, 'inbox');
  const by = Object.fromEntries(items.map((i) => [i.name, i]));

  assert.equal(by['photo.JPG'].dest, path.join(inbox, '이미지', 'photo.JPG'), '확장자는 대소문자 무시');
  assert.equal(by['연봉 계약서.pdf'].dest, path.join(archive, '연봉 계약서.pdf'), '먼저 나온 규칙이 이긴다');
  assert.equal(by['report.pdf'].note, 'renamed');
  assert.equal(by['report.pdf'].dest, path.join(inbox, '문서', 'report (2).pdf'));
  assert.equal(by['same.pdf'].action, 'trash');
  assert.equal(by['same.pdf'].note, 'duplicate');
  assert.equal(by['old-installer.dmg'].action, 'trash');
  assert.ok(!by['new-installer.dmg'], '7일 안 된 설치 파일은 그대로');
  assert.deepEqual(unmatched.map((f) => f.name).sort(), ['mystery.bin', 'new-installer.dmg']);
  assert.ok(!items.concat(unmatched).some((f) => f.name.startsWith('.') || f.name.endsWith('.crdownload')));
});

test('apply → undo: 옮긴 파일은 원래 자리로 돌아온다', () => {
  const cfg = setup();
  const moves = scanTarget(cfg, 'inbox').items.filter((i) => i.action === 'move');
  const { results, run } = applyPlan(cfg, moves, 'test');
  assert.ok(results.every((r) => r.ok), JSON.stringify(results));
  assert.ok(fs.existsSync(path.join(inbox, '이미지', 'photo.JPG')));
  assert.ok(fs.existsSync(path.join(inbox, '문서', 'report (2).pdf')));
  assert.equal(fs.readFileSync(path.join(inbox, '문서', 'report.pdf'), 'utf8'), 'old report', '기존 파일은 덮어쓰지 않는다');
  assert.equal(loadHistory().at(-1).id, run.id);

  const undo = undoLast();
  assert.equal(undo.restored, moves.length);
  assert.ok(fs.existsSync(path.join(inbox, 'photo.JPG')));
  assert.ok(fs.existsSync(path.join(inbox, '연봉 계약서.pdf')));
  assert.equal(undoLast().ok, false, '두 번 되돌리지 않는다');
});

test('apply: 실행 직전에 목적지가 생기면 덮어쓰지 않고 이름을 바꾼다', () => {
  const cfg = setup();
  const item = scanTarget(cfg, 'inbox').items.find((i) => i.name === 'photo.JPG');
  write(path.join(inbox, '이미지', 'photo.JPG'), 'someone else');
  const { results } = applyPlan(cfg, [item], 'test');
  assert.ok(results[0].ok);
  assert.equal(fs.readFileSync(path.join(inbox, '이미지', 'photo.JPG'), 'utf8'), 'someone else');
  assert.ok(fs.existsSync(path.join(inbox, '이미지', 'photo (2).JPG')));
});

test('apply: 중복 원본이 사라졌으면 버리지 않고 원본 자리로 옮긴다', () => {
  const cfg = setup();
  const dup = scanTarget(cfg, 'inbox').items.find((i) => i.name === 'same.pdf');
  assert.equal(dup.note, 'duplicate');
  fs.unlinkSync(path.join(inbox, '문서', 'same.pdf'));
  const { results } = applyPlan(cfg, [dup], 'test');
  assert.ok(results[0].ok, results[0].message);
  assert.equal(fs.readFileSync(path.join(inbox, '문서', 'same.pdf'), 'utf8'), 'identical');
});

test('planFromMoves / applyPlan: 정리 대상 밖은 거부한다', () => {
  const cfg = setup();
  const outside = path.join(sandbox, 'outside.txt');
  write(outside, 'x');
  const { items, rejected } = planFromMoves(cfg, [
    { src: outside, destFolder: archive },
    { src: path.join(inbox, 'mystery.bin'), destFolder: os.homedir() },
    { src: path.join(inbox, 'mystery.bin'), destFolder: '기타' },
  ]);
  assert.equal(rejected.length, 2);
  assert.equal(items.length, 1);
  assert.equal(items[0].dest, path.join(inbox, '기타', 'mystery.bin'));

  const forged = applyPlan(cfg, [{ id: 'x', src: outside, action: 'move', dest: path.join(archive, 'outside.txt') }]);
  assert.equal(forged.results[0].ok, false);
  assert.ok(fs.existsSync(outside));
});

test('trash: 휴지통으로 보낸다', { skip: process.platform === 'linux' && !process.env.CI }, () => {
  const cfg = setup();
  const item = scanTarget(cfg, 'inbox').items.find((i) => i.name === 'same.pdf');
  const { results } = applyPlan(cfg, [item], 'test');
  assert.ok(results[0].ok, results[0].message);
  assert.ok(!fs.existsSync(path.join(inbox, 'same.pdf')));
  assert.ok(fs.existsSync(path.join(inbox, '문서', 'same.pdf')), '원본은 남아 있다');
});

test('Claude 응답 해석: structured_output과 텍스트 JSON 모두', () => {
  const s = [{ file: 'a.pdf', dest: '~/Documents', reason: 'x' }];
  assert.deepEqual(parseSuggestions(JSON.stringify({ structured_output: { suggestions: s } })), s);
  assert.deepEqual(parseSuggestions(JSON.stringify({ result: `여기요\n${JSON.stringify({ suggestions: s })}` })), s);
  assert.throws(() => parseSuggestions(JSON.stringify({ is_error: true, result: 'Not logged in' })), /Not logged in/);
  const prompt = buildPrompt(inbox, [{ name: 'a.pdf', size: 2048, mtimeMs: 0 }], [inbox]);
  assert.match(prompt, /a\.pdf \| 2KB \| 1970-01-01/);
});

test('MCP 서버: initialize → tools/list → scan_folder', async () => {
  setup();
  const child = spawn(process.execPath, [path.join(__dirname, '..', 'src', 'mcp', 'server.js')], { env: process.env });
  const replies = new Map();
  let buf = '';
  child.stdout.on('data', (d) => {
    buf += d;
    let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      const msg = JSON.parse(buf.slice(0, i));
      buf = buf.slice(i + 1);
      replies.get(msg.id)?.(msg);
    }
  });
  let seq = 0;
  const rpc = (method, params) =>
    new Promise((resolve) => {
      const id = ++seq;
      replies.set(id, resolve);
      child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    });

  const init = await rpc('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 't', version: '0' } });
  assert.equal(init.result.protocolVersion, '2025-06-18');
  child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`);
  const list = await rpc('tools/list', {});
  assert.deepEqual(list.result.tools.map((t) => t.name), ['list_targets', 'scan_folder', 'apply_plan', 'add_rule', 'undo_last', 'history']);

  const scan = JSON.parse((await rpc('tools/call', { name: 'scan_folder', arguments: { target: 'inbox' } })).result.content[0].text);
  const photo = scan.plan.find((p) => p.file.endsWith('photo.JPG'));
  assert.ok(photo);
  assert.ok(scan.unmatched.some((u) => u.file.endsWith('mystery.bin')));

  const applied = JSON.parse(
    (await rpc('tools/call', {
      name: 'apply_plan',
      arguments: { item_ids: [photo.id], moves: [{ src: path.join(inbox, 'mystery.bin'), dest_folder: archive, reason: '테스트' }] },
    })).result.content[0].text,
  );
  assert.ok(applied.results.every((r) => r.ok), JSON.stringify(applied));
  assert.ok(fs.existsSync(path.join(archive, 'mystery.bin')));

  const added = await rpc('tools/call', { name: 'add_rule', arguments: { target: 'inbox', name: '바이너리', extensions: ['bin'], action: 'move', dest: '기타' } });
  assert.ok(!added.result.isError);
  assert.equal(loadConfig().targets[0].rules[0].name, '바이너리');

  const bad = await rpc('tools/call', { name: 'scan_folder', arguments: { target: '/nope' } });
  assert.equal(bad.result.isError, true);
  child.stdin.end();
});

test('testRule·missingDirs: 규칙 미리보기와 새 폴더 계산', () => {
  const cfg = setup();
  assert.deepEqual(testRule(cfg, 'inbox', { match: { extensions: ['pdf'] } }).sort(), ['report.pdf', 'same.pdf', '연봉 계약서.pdf']);
  assert.deepEqual(testRule(cfg, 'inbox', { match: {} }), [], '조건 없으면 아무것도 안 잡힌다');
  const dest = path.join(inbox, '새폴더', '하위', 'a.txt');
  assert.deepEqual(missingDirs(dest), [path.join(inbox, '새폴더', '하위'), path.join(inbox, '새폴더')]);
  assert.deepEqual(missingDirs(path.join(inbox, '문서', 'a.txt')), []);
});

test('AutoTidy: 다 받아진 파일만 자동으로 정리한다', async () => {
  const cfg = setup();
  cfg.targets[0].auto = true;
  cfg.settings = { ...cfg.settings, autoDelaySec: 3 };
  saveConfig(cfg);
  const runs = [];
  const auto = new AutoTidy({ load: loadConfig, onRun: (t, r) => runs.push(r) });
  try {
    auto.sync();
    // 첫 확인: 크기를 기록만 하고 기다린다.
    assert.equal(auto.check('inbox'), null);
    assert.ok(fs.existsSync(path.join(inbox, 'photo.JPG')));
    // 기다리는 동안 새로 받는 중인 파일(방금 수정됨)
    write(path.join(inbox, 'downloading.png'), 'partial');
    await new Promise((r) => setTimeout(r, 3200));
    auto.check('inbox');
    assert.ok(runs.length >= 1, '정리가 실행됐다');
    assert.ok(fs.existsSync(path.join(inbox, '이미지', 'photo.JPG')));
    assert.ok(!fs.existsSync(path.join(inbox, 'old-installer.dmg')), '오래된 설치 파일은 휴지통으로');
    assert.ok(fs.existsSync(path.join(inbox, 'downloading.png')), '아직 받는 중인 파일은 기다린다');
    assert.equal(loadHistory().at(-1).source, 'auto');
    // 끄면 감시를 멈춘다.
    cfg.targets[0].auto = false;
    assert.deepEqual(auto.sync(saveConfig(cfg)), []);
  } finally {
    auto.stop();
  }
});

test.after(() => fs.rmSync(sandbox, { recursive: true, force: true }));
