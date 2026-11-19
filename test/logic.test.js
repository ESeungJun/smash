'use strict';
// 화면의 순수 계산 시험. `node --test test/`
const test = require('node:test');
const assert = require('node:assert/strict');
const L = require('../src/renderer/logic.js');

const rule = (id, extra = {}) => ({ id, name: id, enabled: true, match: { extensions: ['pdf'], nameIncludes: [], olderThanDays: 0 }, action: 'move', dest: 'x', ...extra });
const cfg = (targets) => ({ version: 3, targets, settings: { sound: true } });
const target = (id, rules, extra = {}) => ({ id, path: `~/${id}`, auto: false, rules, ...extra });

test('설정 합치기: 밖에서 넣은 규칙은 살리고 화면에서 고친 것이 이긴다', () => {
  const base = cfg([target('dl', [rule('a'), rule('b')])]);
  // 화면: b 를 지우고 a 이름을 바꿨다. 그 사이 MCP 가 맨 위에 m1, 맨 아래에 m2 를 넣었다.
  const mine = cfg([target('dl', [rule('a', { name: '고친 이름' })])]);
  const theirs = cfg([target('dl', [rule('m1'), rule('a'), rule('b'), rule('m2')])]);
  const out = L.mergeConfig(base, mine, theirs);
  assert.deepEqual(out.targets[0].rules.map((r) => r.id), ['m1', 'a', 'm2']);
  assert.equal(out.targets[0].rules[1].name, '고친 이름');
  assert.equal(mine.targets[0].rules.length, 1, '입력을 고치지 않는다');
});

test('설정 합치기: 화면에서 뺀 폴더는 되살리지 않고, 밖에서 더한 폴더는 넣는다', () => {
  const base = cfg([target('dl', [rule('a')]), target('docs', [])]);
  const mine = cfg([target('dl', [rule('a')])]);
  const theirs = cfg([target('dl', [rule('a')]), target('docs', [rule('z')]), target('desk', [rule('n')])]);
  const out = L.mergeConfig(base, mine, theirs);
  assert.deepEqual(out.targets.map((t) => t.id), ['dl', 'desk']);
  assert.deepEqual(L.mergeConfig(base, mine, null), mine, '디스크 것을 못 읽으면 화면 것 그대로');
  assert.deepEqual(L.mergeConfig(null, mine, mine).targets.map((t) => t.id), ['dl'], '기준이 없어도 겹치게 넣지 않는다');
});

test('설정 합치기: 같은 규칙을 두 번 넣지 않는다', () => {
  const base = cfg([target('dl', [rule('a')])]);
  const mine = cfg([target('dl', [rule('m1'), rule('a')])]);
  const theirs = cfg([target('dl', [rule('m1'), rule('a')])]);
  assert.deepEqual(L.mergeConfig(base, mine, theirs).targets[0].rules.map((r) => r.id), ['m1', 'a']);
});

test('규칙 목적지가 정리할 폴더 안인지', () => {
  const targets = [{ path: '~/Downloads' }, { path: '~/Documents' }];
  const inside = (dest, owner = '~/Downloads') => L.destInsideTargets(dest, owner, targets, '/Users/me', '/');
  assert.equal(inside('문서'), true, '상대 경로는 그 폴더 기준');
  assert.equal(inside('~/Documents/자료'), true);
  assert.equal(inside('/Users/me/Documents'), true);
  assert.equal(inside('~/Archive'), false);
  assert.equal(inside('../Archive'), false, '.. 로 빠져나가면 밖');
  assert.equal(inside('~/DocumentsBackup'), false, '이름이 앞부분만 같은 폴더는 밖');
  assert.equal(inside(''), true, '아직 안 고른 것은 따로 안내한다');
  assert.equal(L.destInsideTargets('D:\\Archive', 'C:\\Users\\me\\Downloads', [{ path: 'C:\\Users\\me\\Downloads' }], 'C:\\Users\\me', '\\'), false);
  assert.equal(L.destInsideTargets('사진', 'C:\\Users\\me\\Downloads', [{ path: '~/Downloads' }], 'C:\\Users\\me', '\\'), true);
  assert.equal(L.resolvePath('~/Downloads', 'a/../b/./c', '/Users/me', '/'), '/Users/me/Downloads/b/c');
});

// 아래 기대값은 2026-11-17 리팩토링 전 app.js 의 destText·expand·contract·baseName 이 낸 값을 그대로 옮긴 것(특성 테스트).
test('경로 표기: 홈 줄이기·펼치기·마지막 이름', () => {
  const mac = ['/Users/me', '/'];
  assert.deepEqual(['~', '~/a/b', '~x', '/abs/p', 'rel/p', ''].map((p) => L.toNativePath(p, ...mac)), ['/Users/me', '/Users/me/a/b', '~x', '/abs/p', 'rel/p', '']);
  assert.deepEqual(['/Users/me', '/Users/me/a/b', '/Users/mex', '/other', ''].map((p) => L.contractPath(p, ...mac)), ['~', '~/a/b', '/Users/mex', '/other', '']);
  const win = ['C:\\Users\\me', '\\'];
  assert.deepEqual(['~', '~/a/b', '/abs/p', 'rel/p'].map((p) => L.toNativePath(p, ...win)), ['C:\\Users\\me', 'C:\\Users\\me\\a\\b', '\\abs\\p', 'rel\\p']);
  assert.deepEqual(['C:\\Users\\me', 'C:\\Users\\me\\a\\b', 'C:\\Users\\mex'].map((p) => L.contractPath(p, ...win)), ['~', '~/a/b', 'C:\\Users\\mex']);
  assert.deepEqual(['/a/b/', 'C:\\x\\y', 'name', '', '/'].map(L.baseName), ['b', 'y', 'name', '', '/']);
});

test('규칙 목적지 표기: 정리 대상 폴더 이름 기준, 밖이면 ~ 경로', () => {
  const targets = [{ id: 'dl', path: '~/Downloads' }, { id: 'docs', path: '~/Documents' }];
  const [dl, docs] = targets;
  const label = (dest, owner = dl) => L.destLabel(dest, owner, targets, '/Users/me', '/');
  const cases = [
    ['', dl, ''],
    ['자료', dl, 'Downloads/자료'],
    ['자료/계약서', dl, 'Downloads/자료/계약서'],
    ['~/Documents', dl, 'Documents'],
    ['~/Documents/자료', dl, 'Documents/자료'],
    ['/Users/me/Documents/a', dl, 'Documents/a'],
    ['~/Archive', dl, '~/Archive'],
    ['~/DocumentsBackup', dl, '~/DocumentsBackup'],
    ['C:/x', dl, 'C:/x'],
    ['계약서', docs, 'Documents/계약서'],
    ['/Users/me/Downloads', docs, 'Downloads'],
    ['자료', null, '/자료'],
    // resolvePath 와 다른 곳: `.`·`..`·겹친 `/`·끝 `/` 를 정리하지 않고, `\` 로 시작하면 상대 경로로 본다
    ['../Archive', dl, 'Downloads/../Archive'],
    ['사진/../받은 것/', dl, 'Downloads/사진/../받은 것/'],
    ['~/Documents/영수증/', dl, 'Documents/영수증/'],
    ['./a', dl, 'Downloads/./a'],
    ['a//b', dl, 'Downloads/a//b'],
    ['\\x', dl, 'Downloads/\\x'],
  ];
  for (const [dest, owner, want] of cases) assert.equal(label(dest, owner), want, dest);

  const winTargets = [{ path: '~/Downloads' }, { path: 'C:\\Work' }];
  const win = (dest) => L.destLabel(dest, winTargets[0], winTargets, 'C:\\Users\\me', '\\');
  assert.equal(win('자료/계약서'), 'Downloads/자료/계약서');
  assert.equal(win('~/Downloads/x'), 'Downloads/x');
  assert.equal(win('C:\\Work\\a\\b'), 'Work/a/b');
  assert.equal(win('C:/Work/a'), 'Work/a');
  assert.equal(win('D:\\x'), 'D:\\x');
  assert.equal(win('\\x'), 'Downloads//x');
  assert.equal(win('..\\Up'), 'Downloads/../Up');
});

test('규칙 목적지 표기는 resolvePath 로 바꿀 수 없다: `..` 가 있으면 보이는 글자가 다르다', () => {
  // 실제 이동(Rust config::resolve)과 "밖" 판정(destInsideTargets)은 `..` 를 정리한다. 표기는 정리하지 않는다.
  const owner = { path: '~/Downloads' };
  const shown = L.destLabel('../Archive', owner, [owner], '/Users/me', '/');
  const resolved = L.resolvePath(owner.path, '../Archive', '/Users/me', '/');
  assert.equal(shown, 'Downloads/../Archive');
  assert.equal(resolved, '/Users/me/Archive');
  assert.equal(L.destInsideTargets('../Archive', owner.path, [owner], '/Users/me', '/'), false);
});

test('타격음: 철썩이 찰싹보다 크고 낮고 길다. 알림음보다 크지 않다. 약한 단계는 없다(#7)', () => {
  const [slap, wham] = ['slap', 'wham'].map(L.slapSpec);
  assert.ok(slap.gain < wham.gain);
  assert.ok(wham.gain <= 0.5, '최대 음량 상한');
  assert.ok(slap.gain >= 0.4, '찰싹도 충분히 크다');
  assert.ok(slap.bandHz > wham.bandHz && slap.thumpHz[1] > wham.thumpHz[1]);
  assert.ok(slap.tailMs < wham.tailMs && wham.tailMs <= 200, '짧은 소리');
  assert.ok(slap.bursts >= 2, '짝: 짧은 터짐이 여러 번');
  assert.deepEqual(L.slapSpec('tap'), slap, '옛 tap 은 찰싹으로');
  assert.deepEqual(L.slapSpec('nope'), slap);
});

test('행이 빠지는 순서: 30ms 간격, 0.9초를 넘지 않는다', () => {
  assert.deepEqual(L.staggerDelays(4), [0, 30, 60, 90]);
  const many = L.staggerDelays(500);
  assert.equal(many.length, 500);
  assert.equal(Math.max(...many), 900);
  assert.deepEqual(L.staggerDelays(0), []);
});

test('기록 한 줄의 상태와 되돌리기 버튼', () => {
  const mv = (undone = false) => ({ type: 'move', from: 'a', to: 'b', undone });
  assert.deepEqual(L.runStatus({ ops: [mv()], left: 1 }), { key: 'done', canUndo: true, left: 1 });
  assert.deepEqual(L.runStatus({ ops: [mv(true)], undone: true, tried: true, left: 0 }), { key: 'undone', canUndo: false, left: 0 });
  assert.equal(L.runStatus({ ops: [mv(true), mv()], tried: true, left: 1 }).key, 'partial');
  assert.equal(L.runStatus({ ops: [mv(true), mv()], tried: true, left: 1 }).canUndo, true, '실패분은 다시 시도할 수 있다');
  assert.deepEqual(L.runStatus({ ops: [mv()], state: 'interrupted', left: 1 }), { key: 'interrupted', canUndo: true, left: 1 });
  assert.equal(L.runStatus({ ops: [mv()], state: 'running', left: 1 }).canUndo, false);
  assert.equal(L.runStatus({ ops: [], failed: [{ file: 'x', reason: 'y' }] }).canUndo, false, '옮긴 게 없으면 되돌릴 것도 없다');
  // 위치를 모르는 휴지통 파일만 있는 실행: 한 번은 눌러서 안내를 받을 수 있다
  assert.equal(L.runStatus({ ops: [{ type: 'trash', from: 'a' }] }).canUndo, true);
  assert.equal(L.runStatus({ ops: [{ type: 'trash', from: 'a' }] }).left, 0);
});

test('실패 목록과 크기 표기', () => {
  const results = [{ name: 'a', ok: true, message: '→ x' }, { name: 'b', ok: false, message: '파일이 아닙니다' }];
  assert.deepEqual(L.failures(results), [{ name: 'b', reason: '파일이 아닙니다' }]);
  assert.deepEqual(L.failures(undefined), []);
  assert.deepEqual([1.2e9, 3.4e6, 10].map(L.fmtBytes), ['1.2GB', '3MB', '1KB']);
});
