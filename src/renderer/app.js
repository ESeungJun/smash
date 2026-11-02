'use strict';
/* global tidy */

const $ = (sel) => document.querySelector(sel);
// 화면 언어. main이 설정(system이면 OS 언어)을 보고 정해서 config:get으로 넘겨준다.
let LANG = 'ko';
const L = (ko, en) => (LANG === 'ko' ? ko : en);
const esc = (s) =>
  String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

const state = {
  config: null,
  home: '',
  sep: '/',
  categories: [],
  targetId: null,
  scan: null,
  aiItems: [],
  aiNoSuggestion: [],
  unchecked: new Set(),
  collapsed: new Set(),
  folderItems: new Map(), // 트리 폴더 경로 → 그 아래 항목 id들
  busy: false,
  draft: null, // 편집 중인 규칙
  folders: [],
  integration: null,
};

// ---------- 공통 ----------

let toastTimer;
function toast(msg, ms = 2800) {
  const el = $('#toast');
  el.textContent = msg;
  el.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove('show'), ms);
}

function fmtSize(n) {
  if (n >= 1 << 30) return `${(n / (1 << 30)).toFixed(1)}GB`;
  if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(1)}MB`;
  return `${Math.max(1, Math.round(n / 1024))}KB`;
}

const baseName = (p) => String(p).split(/[\\/]/).filter(Boolean).pop() || p;
const dirOf = (p) => p.slice(0, p.lastIndexOf(state.sep));
const expand = (p) => String(p).replace(/^~(?=[\\/]|$)/, state.home).split('/').join(state.sep);
function contract(p) {
  if (!p) return '';
  if (p === state.home) return '~';
  if (p.startsWith(state.home + state.sep)) return `~${p.slice(state.home.length).split(state.sep).join('/')}`;
  return p;
}

const currentTarget = () => state.config.targets.find((t) => t.id === state.targetId);
const targetName = (t) => baseName(t.path);
const cliReady = () => state.integration?.cli?.installed && !state.integration.cli.error;

// 16px 선 아이콘. 색은 currentColor를 따른다.
const ICONS = {
  folder: '<path d="M2 4.5c0-.6.4-1 1-1h3.2l1.5 1.6H13c.6 0 1 .4 1 1V12c0 .6-.4 1-1 1H3c-.6 0-1-.4-1-1z"/>',
  file: '<path d="M4 2h5l3 3v8.5c0 .3-.2.5-.5.5h-7a.5.5 0 0 1-.5-.5v-11c0-.3.2-.5.5-.5z"/><path d="M9 2v3h3"/>',
  trash: '<path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.6c0 .5.4.9.9.9h4c.5 0 .9-.4.9-.9l.6-8.6"/>',
};
const ico = (name) =>
  `<svg class="ico" viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" stroke-linecap="round" aria-hidden="true">${ICONS[name]}</svg>`;

async function saveConfig() {
  state.config = await tidy.saveConfig(state.config);
  renderTargets();
}

function switchTab(name) {
  document.querySelectorAll('.tab').forEach((t) => t.classList.toggle('active', t.dataset.tab === name));
  document.querySelectorAll('.panel').forEach((p) => p.classList.toggle('active', p.id === `tab-${name}`));
  if (name === 'history') renderHistory();
  if (name === 'settings') renderSettings();
  if (name === 'rules') renderRules();
}

// ---------- 폴더 목록 ----------

function renderTargets() {
  $('#targets').innerHTML = state.config.targets
    .map(
      (t) => `<button class="target-btn ${t.id === state.targetId ? 'active' : ''}" data-id="${esc(t.id)}" title="${esc(t.path)}">
        <span>${esc(targetName(t))}</span>${t.auto ? `<span class="auto-badge" title="${L('자동 정리 켜짐', 'Auto-tidy on')}">${L('자동', 'Auto')}</span>` : ''}
      </button>`,
    )
    .join('');
}

async function selectTarget(id) {
  state.targetId = id;
  state.draft = null;
  renderTargets();
  const t = currentTarget();
  $('#target-path').textContent = t ? targetName(t) : L('폴더를 추가해 주세요', 'Add a folder');
  $('#target-path').title = t ? t.path : '';
  renderRules();
  await doScan();
}

async function addTarget() {
  const picked = await tidy.pickFolder(state.home);
  if (!picked) return;
  const label = contract(picked);
  const exists = state.config.targets.find((t) => t.path === label);
  if (exists) return selectTarget(exists.id);
  const id = `t_${Date.now().toString(36)}`;
  state.config.targets.push({ id, path: label, auto: false, rules: [] });
  await saveConfig();
  await selectTarget(id);
  toast(L('폴더를 추가했어요. 규칙 탭에서 정리 규칙을 만들어 보세요.', 'Folder added. Make some rules in the Rules tab.'));
}

let removeArmed = false;
async function removeTarget() {
  const btn = $('#remove-target');
  if (!removeArmed) {
    removeArmed = true;
    btn.textContent = L('한 번 더 누르면 목록에서 뺍니다 (파일은 그대로)', 'Click again to remove it from the list (files stay put)');
    setTimeout(() => {
      removeArmed = false;
      btn.textContent = L('이 폴더를 목록에서 빼기', 'Remove this folder from the list');
    }, 3000);
    return;
  }
  removeArmed = false;
  btn.textContent = L('이 폴더를 목록에서 빼기', 'Remove this folder from the list');
  state.config.targets = state.config.targets.filter((t) => t.id !== state.targetId);
  await saveConfig();
  state.targetId = state.config.targets[0]?.id || null;
  if (state.targetId) await selectTarget(state.targetId);
  else {
    state.scan = null;
    renderTargets();
    renderPreview();
  }
}

// ---------- 미리보기 (트리) ----------

async function doScan() {
  if (!state.targetId) return;
  state.aiItems = [];
  state.aiNoSuggestion = [];
  state.unchecked = new Set();
  $('#plan').innerHTML = `<div class="block"><div class="empty"><span class="spinner"></span>${L('살펴보는 중…', 'Looking…')}</div></div>`;
  $('#unmatched').innerHTML = '';
  try {
    state.scan = await tidy.scan(state.targetId);
  } catch (err) {
    state.scan = null;
    $('#plan').innerHTML = `<div class="block"><div class="empty">${esc(err.message)}</div></div>`;
    $('#target-sub').textContent = '';
    updateApplyButton();
    return;
  }
  renderPreview();
}

const allPlanItems = () => (state.scan ? [...state.scan.items, ...state.aiItems] : []);
const checkedItems = () => allPlanItems().filter((it) => !state.unchecked.has(it.id));

function newNode(name, abs) {
  return { name, abs, children: new Map(), items: [] };
}

// 목적지 기준 트리. 정리 대상 폴더(Documents 등)를 뿌리로 삼는다.
function buildTree(items) {
  const roots = state.config.targets
    .map((t) => ({ abs: expand(t.path), name: targetName(t) }))
    .sort((a, b) => b.abs.length - a.abs.length);
  const tops = new Map();
  const trash = [];
  for (const it of items) {
    if (it.action === 'trash') {
      trash.push(it);
      continue;
    }
    const dir = dirOf(it.dest);
    const root = roots.find((r) => dir === r.abs || dir.startsWith(r.abs + state.sep));
    const rootAbs = root ? root.abs : dir;
    if (!tops.has(rootAbs)) tops.set(rootAbs, newNode(root ? root.name : contract(dir), rootAbs));
    let node = tops.get(rootAbs);
    const rel = dir === rootAbs ? [] : dir.slice(rootAbs.length + 1).split(state.sep);
    for (const seg of rel) {
      const abs = node.abs + state.sep + seg;
      if (!node.children.has(seg)) node.children.set(seg, newNode(seg, abs));
      node = node.children.get(seg);
    }
    node.items.push(it);
  }
  return { tops: [...tops.values()], trash };
}

function collectIds(node) {
  const ids = node.items.map((i) => i.id);
  for (const child of node.children.values()) ids.push(...collectIds(child));
  return ids;
}

function fileRow(it, depth) {
  const off = state.unchecked.has(it.id);
  const isAi = !it.ruleId && String(it.reason || '').startsWith('AI');
  const fromDir = baseName(dirOf(it.src));
  const sameDir = it.action !== 'trash' && dirOf(it.src) === dirOf(it.dest);
  const tags = [
    isAi ? `<span class="tag ai">${L('AI 제안', 'AI pick')}</span>` : '',
    it.note === 'duplicate' ? `<span class="tag warn">${L('중복', 'Duplicate')}</span>` : '',
    it.note === 'renamed'
      ? `<span class="tag warn" title="${L(`같은 이름이 있어 ${esc(baseName(it.dest))}(으)로 저장`, `Saved as ${esc(baseName(it.dest))} because the name is taken`)}">${L('이름 변경', 'Renamed')}</span>`
      : '',
  ].join('');
  const where =
    it.note === 'duplicate'
      ? L(`같은 파일이 ${esc(destText(dirOf(it.duplicateOf)))}에 있어요`, `Same file already in ${esc(destText(dirOf(it.duplicateOf)))}`)
      : `${sameDir ? '' : L(`${esc(fromDir)}에서 · `, `from ${esc(fromDir)} · `)}${esc(String(it.reason || '').replace(/^규칙: /, ''))}`;
  return `<li class="trow file ${off ? 'off' : ''}" data-depth="${Math.min(depth, 8)}">
    <span class="caret-space"></span>
    <input type="checkbox" data-item="${esc(it.id)}" ${off ? '' : 'checked'} aria-label="${L(`${esc(it.name)} 포함`, `Include ${esc(it.name)}`)}" />
    <span class="ficon">${ico('file')}</span>
    <span class="tname" title="${esc(it.srcLabel)}">${esc(it.name)}</span>
    <span class="tmeta">${where} · ${fmtSize(it.size)}</span>
    <span class="tags">${tags}</span>
  </li>`;
}

function folderRow(node, depth, newDirs, { icon = 'folder', isNew } = {}) {
  const ids = collectIds(node);
  state.folderItems.set(node.abs, ids);
  const collapsed = state.collapsed.has(node.abs);
  const fresh = isNew ?? newDirs.has(node.abs);
  const children = [...node.children.values()]
    .sort((a, b) => a.name.localeCompare(b.name, 'ko'))
    .map((c) => folderRow(c, depth + 1, newDirs))
    .join('');
  const files = node.items
    .slice()
    .sort((a, b) => a.name.localeCompare(b.name, 'ko'))
    .map((it) => fileRow(it, depth + 1))
    .join('');
  return `<li class="tnode">
    <div class="trow folder${fresh ? ' fresh' : ''}${node.abs === '::trash' ? ' bin' : ''}" data-depth="${Math.min(depth, 8)}">
      <button class="caret ${collapsed ? 'closed' : ''}" data-collapse="${esc(node.abs)}" aria-label="${L('펼치기/접기', 'Expand/collapse')}">▾</button>
      <input type="checkbox" data-folder="${esc(node.abs)}" aria-label="${L(`${esc(node.name)} 전체`, `All of ${esc(node.name)}`)}" />
      <span class="ficon">${ico(icon)}</span>
      <span class="tname folder-name">${esc(node.name)}</span>
      <span class="tmeta">${L(`${ids.length}개`, ids.length)}</span>
      <span class="tags">${fresh ? `<span class="tag new">${L('새 폴더', 'New folder')}</span>` : ''}</span>
    </div>
    ${collapsed ? '' : `<ul class="tchildren">${children}${files}</ul>`}
  </li>`;
}

function syncFolderChecks() {
  document.querySelectorAll('input[data-folder]').forEach((box) => {
    const ids = state.folderItems.get(box.dataset.folder) || [];
    const on = ids.filter((id) => !state.unchecked.has(id)).length;
    box.checked = on > 0 && on === ids.length;
    box.indeterminate = on > 0 && on < ids.length;
  });
}

function renderPreview() {
  const scan = state.scan;
  if (!scan) {
    $('#plan').innerHTML = '';
    $('#unmatched').innerHTML = '';
    updateApplyButton();
    return;
  }
  const items = allPlanItems();
  const aiNames = new Set(state.aiItems.map((i) => i.name));
  const unmatched = scan.unmatched.filter((f) => !aiNames.has(f.name));
  const { tops, trash } = buildTree(items);
  const newDirs = new Set(items.flatMap((i) => i.newDirs || []));
  const moving = items.length - trash.length;
  $('#target-sub').textContent = [
    scan.rootLabel,
    L(`옮길 파일 ${moving}개`, `${moving} to move`),
    trash.length ? L(`휴지통 ${trash.length}개`, `${trash.length} to Trash`) : '',
    newDirs.size ? L(`새 폴더 ${newDirs.size}개`, `${newDirs.size} new folders`) : '',
    unmatched.length ? L(`규칙 밖 ${unmatched.length}개`, `${unmatched.length} unmatched`) : '',
  ]
    .filter(Boolean)
    .join(' · ');

  state.folderItems = new Map();
  const trashNode = { name: L('휴지통', 'Trash'), abs: '::trash', children: new Map(), items: trash };
  const tree = [
    ...tops.map((n) => folderRow(n, 0, newDirs)),
    trash.length ? folderRow(trashNode, 0, newDirs, { icon: 'trash', isNew: false }) : '',
  ].join('');

  $('#plan').innerHTML = `<div class="block">
    <div class="block-head">
      <div>
        <div class="block-title">${L('정리하면 이렇게 돼요', 'After tidying')}<span class="count">${items.length}</span></div>
        <div class="block-sub">${L('폴더 옆 ▾를 누르면 접고 펼 수 있어요. 체크를 풀면 그 파일은 그대로 둬요.', 'Click ▾ next to a folder to collapse it. Uncheck a file to leave it where it is.')}</div>
      </div>
      ${items.length ? `<button class="link" id="toggle-all">${L('전체 선택/해제', 'Select all / none')}</button>` : ''}
    </div>
    ${
      items.length
        ? `<ul class="tree">${tree}</ul>`
        : L(
            '<div class="empty big"><b>치울 게 없어요.</b><br />규칙에 맞는 파일이 하나도 없어요. 엄마가 칭찬하겠네요.</div>',
            '<div class="empty big"><b>Nothing to tidy.</b><br />No files match your rules. Mom would be proud.</div>',
          )
    }
  </div>`;
  syncFolderChecks();

  const aiButton = !unmatched.length
    ? ''
    : state.integration && !cliReady()
      ? `<button class="ghost small" data-goto="settings">${L('Claude 연결하기', 'Connect Claude')}</button>`
      : `<button class="ghost small" id="ai-suggest">${L('Claude에게 자리 물어보기', 'Ask Claude where they go')}</button>`;
  $('#unmatched').innerHTML = unmatched.length
    ? `<div class="block">
        <div class="block-head">
          <div>
            <div class="block-title">${L('규칙에 안 걸린 파일', 'Not matched by any rule')}<span class="count">${unmatched.length}</span></div>
            <div class="block-sub">${L('Claude에게는 파일 이름·크기·날짜와 폴더 목록만 보내요. 파일 내용은 안 보내요.', 'Claude only gets file names, sizes, dates and your folder list, never file contents.')}</div>
          </div>
          ${aiButton}
        </div>
        <ul class="rows">${unmatched
          .map(
            (f) => `<li class="plain-row"><span class="ficon">${ico('file')}</span><span class="fname" title="${esc(f.srcLabel)}">${esc(f.name)}</span><span class="dest">${fmtSize(f.size)}</span></li>`,
          )
          .join('')}</ul>
        ${state.aiNoSuggestion.length ? `<div class="note">${L(`Claude가 확신하지 못한 ${state.aiNoSuggestion.length}개는 그대로 둬요.`, `Claude wasn't sure about ${state.aiNoSuggestion.length}, so they stay put.`)}</div>` : ''}
      </div>`
    : '';
  updateApplyButton();
}

function updateApplyButton() {
  const n = checkedItems().length;
  const btn = $('#apply');
  btn.disabled = state.busy || n === 0;
  btn.textContent = n ? L(`짝! 정리하기 (${n})`, `Smack! Tidy up (${n})`) : L('짝! 정리하기', 'Smack! Tidy up');
}

async function aiSuggest() {
  const btn = $('#ai-suggest');
  const aiNames = new Set(state.aiItems.map((i) => i.name));
  const unmatched = state.scan.unmatched.filter((f) => !aiNames.has(f.name));
  btn.disabled = true;
  btn.innerHTML = `<span class="spinner"></span>${L('Claude가 보는 중…', 'Claude is looking…')}`;
  try {
    const res = await tidy.suggest(state.targetId, unmatched);
    state.aiItems.push(...res.items);
    state.aiNoSuggestion = res.noSuggestion;
    renderPreview();
    toast(
      res.items.length
        ? L(`Claude가 ${res.items.length}개 파일의 자리를 골랐어요. 트리에서 확인하세요.`, `Claude picked places for ${res.items.length} files. Check them in the tree.`)
        : L('Claude도 확신할 만한 자리를 못 찾았어요.', "Claude couldn't find a place it was sure about either."),
    );
  } catch (err) {
    btn.disabled = false;
    btn.textContent = L('Claude에게 자리 물어보기', 'Ask Claude where they go');
    toast(L(`제안 실패: ${err.message}`, `Suggestion failed: ${err.message}`), 6000);
  }
}

async function doApply() {
  const items = checkedItems();
  if (!items.length) return;
  state.busy = true;
  updateApplyButton();
  $('#apply').innerHTML = `<span class="spinner"></span>${L('정리 중…', 'Tidying…')}`;
  try {
    const { results } = await tidy.apply(items);
    const ok = results.filter((r) => r.ok).length;
    const failed = results.filter((r) => !r.ok);
    toast(
      failed.length
        ? L(`${ok}개 정리, ${failed.length}개 실패: ${failed[0].name} — ${failed[0].message}`, `Tidied ${ok}, ${failed.length} failed: ${failed[0].name} — ${failed[0].message}`)
        : L(`짝! ${ok}개 치웠어요. 기록 탭에서 되돌릴 수 있어요.`, `Smack! Tidied ${ok} files. You can undo it in the History tab.`),
      failed.length ? 6000 : 3500,
    );
  } catch (err) {
    toast(L(`실행 실패: ${err.message}`, `Failed: ${err.message}`), 6000);
  } finally {
    state.busy = false;
    await doScan();
  }
}

// ---------- 규칙 ----------

function chipList(values, max = 5) {
  const shown = values.slice(0, max).map((v) => `<span class="chip">${esc(v)}</span>`).join('');
  return values.length > max ? `${shown}<span class="chip more">+${values.length - max}</span>` : shown;
}

function destText(dest) {
  if (!dest) return '';
  const t = currentTarget();
  const rootAbs = t ? expand(t.path) : '';
  const isAbs = dest.startsWith('~') || dest.startsWith('/') || /^[A-Za-z]:/.test(dest);
  const full = isAbs ? expand(dest) : rootAbs + state.sep + dest.split('/').join(state.sep);
  for (const target of state.config.targets) {
    const r = expand(target.path);
    if (full === r) return targetName(target);
    if (full.startsWith(r + state.sep)) return `${targetName(target)}/${full.slice(r.length + 1).split(state.sep).join('/')}`;
  }
  return contract(full);
}

function ruleSentence(r) {
  const parts = [];
  if (r.match.olderThanDays) parts.push(`<span class="chip soft">${L(`${r.match.olderThanDays}일 지난`, `older than ${r.match.olderThanDays} days`)}</span>`);
  if (r.match.nameIncludes.length) parts.push(L(`이름에 ${chipList(r.match.nameIncludes.map((w) => `"${w}"`), 3)} 가 들어간`, `with ${chipList(r.match.nameIncludes.map((w) => `"${w}"`), 3)} in the name`));
  if (r.match.extensions.length) parts.push(chipList(r.match.extensions));
  // 영어는 "확장자 files 기간 이름" 순서가 자연스럽다.
  const [ageEn, nameEn, extEn] = [
    r.match.olderThanDays ? parts[0] : '',
    r.match.nameIncludes.length ? parts[r.match.olderThanDays ? 1 : 0] : '',
    r.match.extensions.length ? parts[parts.length - 1] : '',
  ];
  const what = parts.length
    ? L(`${parts.join(' ')} 파일은`, [extEn, 'files', ageEn, nameEn].filter(Boolean).join(' '))
    : `<span class="warn-text">${L('조건이 없어 적용되지 않아요', 'No conditions, so it never applies')}</span>`;
  const how =
    r.action === 'trash'
      ? L('<b class="to trash">휴지통</b>으로 보내요', 'go to the <b class="to trash">Trash</b>')
      : L(`<b class="to">${esc(destText(r.dest))}</b> 로 옮겨요`, `move to <b class="to">${esc(destText(r.dest))}</b>`);
  return `${what} → ${how}`;
}

function ruleCard(r, i, total) {
  return `<div class="rule-card ${r.enabled ? '' : 'disabled'}">
    <div class="rule-body">
      <div class="rule-name">${esc(r.name)}</div>
      <div class="rule-sentence">${ruleSentence(r)}</div>
    </div>
    <div class="rule-side">
      <label class="switch" title="${L('켜기/끄기', 'On/off')}"><input type="checkbox" data-rule-toggle="${esc(r.id)}" ${r.enabled ? 'checked' : ''} /><span></span></label>
      <div class="rule-actions">
        <button class="icon-btn" data-rule-move="${esc(r.id)}" data-dir="-1" ${i === 0 ? 'disabled' : ''} title="${L('먼저 적용', 'Apply earlier')}">↑</button>
        <button class="icon-btn" data-rule-move="${esc(r.id)}" data-dir="1" ${i === total - 1 ? 'disabled' : ''} title="${L('나중에 적용', 'Apply later')}">↓</button>
        <button class="icon-btn" data-rule-edit="${esc(r.id)}">${L('수정', 'Edit')}</button>
        <button class="icon-btn danger" data-rule-delete="${esc(r.id)}">${L('삭제', 'Delete')}</button>
      </div>
    </div>
  </div>`;
}

function newDraft(from) {
  const r = from || { name: '', match: { extensions: [], nameIncludes: [], olderThanDays: 0 }, action: 'move', dest: '' };
  return {
    id: from?.id || null,
    name: r.name,
    exts: [...r.match.extensions],
    words: [...r.match.nameIncludes],
    days: r.match.olderThanDays || 0,
    action: r.action,
    dest: r.dest || '',
    matches: null,
  };
}

const draftRule = (d) => ({
  name: d.name.trim() || L('새 규칙', 'New rule'),
  enabled: true,
  match: { extensions: d.exts, nameIncludes: d.words, olderThanDays: d.days },
  action: d.action,
  dest: d.action === 'trash' ? '' : d.dest.trim(),
});

const dayOptions = () => [
  [0, L('바로', 'Right away')],
  [1, L('하루 지나면', 'After a day')],
  [7, L('일주일 지나면', 'After a week')],
  [30, L('한 달 지나면', 'After a month')],
  [90, L('석 달 지나면', 'After 3 months')],
];

function matchLineHtml(d) {
  const t = currentTarget();
  const m = d.matches;
  const body =
    m == null
      ? `<span class="spinner"></span>${L('확인 중…', 'Checking…')}`
      : m.length
        ? L(`지금 <b>${esc(targetName(t))}</b>에서 맞는 파일 <b>${m.length}개</b>`, `<b>${m.length}</b> matching files in <b>${esc(targetName(t))}</b> right now`) +
          `: ${m.slice(0, 6).map(esc).join(', ')}${m.length > 6 ? ' …' : ''}`
        : L(`지금 <b>${esc(targetName(t))}</b>에는 맞는 파일이 없어요.`, `No matching files in <b>${esc(targetName(t))}</b> right now.`);
  return `<div class="match-line" id="match-line">${body}</div>`;
}

function destPreviewHtml(d) {
  if (!d.dest) return L('옮길 폴더를 골라 주세요.', 'Pick a folder to move them to.');
  const label = destText(d.dest);
  const exists = state.folders.some((f) => destText(f) === label);
  return `→ <b>${esc(label)}</b>${exists ? '' : ` <span class="tag new">${L('새 폴더로 만들어요', 'Will be created')}</span>`}`;
}

function editorHtml() {
  const d = state.draft;
  const catChips = state.categories
    .map((c) => {
      const has = c.extensions.filter((e) => d.exts.includes(e)).length;
      const cls = has === c.extensions.length ? 'on' : has ? 'partial' : '';
      return `<button type="button" class="cat ${cls}" data-cat="${esc(c.key)}">${esc(c.label)}</button>`;
    })
    .join('');
  const tags = (list, kind) =>
    list.map((v) => `<span class="tag-chip">${esc(v)}<button type="button" data-untag="${kind}" data-value="${esc(v)}" aria-label="${L(`${esc(v)} 빼기`, `Remove ${esc(v)}`)}">×</button></span>`).join('');
  return `<div class="editor" id="editor">
    <div class="editor-head">
      <input class="name-input" id="d-name" type="text" placeholder="${L('규칙 이름 (예: 회사 서류)', 'Rule name (e.g. Work papers)')}" value="${esc(d.name)}" />
    </div>

    <div class="step">
      <div class="step-title"><span class="step-no">1</span>${L('어떤 파일을 치울까요?', 'Which files?')}</div>
      <div class="cats">${catChips}</div>
      <div class="field">
        <div class="field-label">${L('확장자', 'Extensions')}</div>
        <div class="tag-box">${tags(d.exts, 'ext')}<input id="d-ext" type="text" placeholder="${d.exts.length ? L('추가…', 'Add…') : L('예: pdf, hwp (엔터)', 'e.g. pdf, docx (Enter)')}" /></div>
      </div>
      <div class="field">
        <div class="field-label">${L('이름에 이 단어가 있으면', 'If the name contains')} <span class="muted">${L('(선택)', '(optional)')}</span></div>
        <div class="tag-box">${tags(d.words, 'word')}<input id="d-word" type="text" placeholder="${L('예: 계약서, 영수증 (엔터)', 'e.g. contract, receipt (Enter)')}" /></div>
      </div>
      <div class="field">
        <div class="field-label">${L('언제?', 'When?')}</div>
        <div class="seg">${dayOptions().map(([v, l]) => `<button type="button" data-days="${v}" class="${d.days === v ? 'on' : ''}">${l}</button>`).join('')}</div>
      </div>
    </div>

    <div class="step">
      <div class="step-title"><span class="step-no">2</span>${L('어떻게 할까요?', 'What should happen?')}</div>
      <div class="seg wide">
        <button type="button" data-action="move" class="${d.action === 'move' ? 'on' : ''}">${L('폴더로 옮기기', 'Move to a folder')}</button>
        <button type="button" data-action="trash" class="${d.action === 'trash' ? 'on' : ''}">${L('휴지통으로 보내기', 'Send to Trash')}</button>
      </div>
      ${
        d.action === 'move'
          ? `<div class="field">
              <div class="inline">
                <input id="d-dest" type="text" list="folder-list" placeholder="${L('폴더 이름이나 경로 (예: ~/Documents/회사)', 'Folder name or path (e.g. ~/Documents/Work)')}" value="${esc(d.dest)}" />
                <button type="button" class="ghost small" id="pick-dest">${L('폴더 고르기…', 'Choose folder…')}</button>
              </div>
              <div class="dest-preview" id="dest-preview">${destPreviewHtml(d)}</div>
            </div>`
          : `<p class="hint">${L('휴지통으로 보내요. 영구 삭제는 하지 않아요.', 'They go to the Trash. Nothing is deleted for good.')}</p>`
      }
    </div>

    ${matchLineHtml(d)}
    <div class="editor-actions">
      <button type="button" class="ghost" id="cancel-rule">${L('취소', 'Cancel')}</button>
      <button type="button" class="primary" id="save-rule">${L('저장', 'Save')}</button>
    </div>
  </div>`;
}

let matchTimer;
function refreshMatches() {
  clearTimeout(matchTimer);
  const d = state.draft;
  if (!d) return;
  matchTimer = setTimeout(async () => {
    if (state.draft !== d) return;
    const rule = draftRule(d);
    const hasCondition = rule.match.extensions.length || rule.match.nameIncludes.length || rule.match.olderThanDays;
    try {
      d.matches = hasCondition ? await tidy.testRule(state.targetId, rule) : [];
    } catch {
      d.matches = [];
    }
    if (state.draft === d && $('#match-line')) $('#match-line').outerHTML = matchLineHtml(d);
  }, 250);
}

function renderEditor(focusId) {
  const host = $('#editor');
  if (host) host.outerHTML = editorHtml();
  if (focusId) $(`#${focusId}`)?.focus();
  refreshMatches();
}

function renderRules() {
  const t = currentTarget();
  if (!t) {
    $('#rules-top').innerHTML = '';
    $('#rules').innerHTML = '';
    return;
  }
  const ai = cliReady();
  $('#rules-top').innerHTML = `<div class="nl-card">
    <div class="nl-title">${L('말로 규칙 만들기', 'Describe a rule')}</div>
    <div class="inline">
      <input id="nl-text" type="text" placeholder="${L("예: 이름에 '계약서'가 들어간 PDF는 문서/계약서로 옮겨줘", "e.g. Move PDFs with 'invoice' in the name to Documents/Invoices")}" ${ai ? '' : 'disabled'} />
      <button class="ghost" id="nl-make" ${ai ? '' : 'disabled'}>${L('Claude로 만들기', 'Make it with Claude')}</button>
    </div>
    <div class="hint">${
      ai
        ? L('Claude가 규칙을 채워 주면 확인하고 저장하면 돼요.', 'Claude fills in the rule. Check it and save.')
        : L('Claude Code CLI가 있으면 말로 규칙을 만들 수 있어요. (자동 정리 · Claude 탭)', 'With the Claude Code CLI you can describe rules in words. (Auto-tidy · Claude tab)')
    }</div>
  </div>
  <div class="panel-head">
    <p class="hint">${L('위에 있는 규칙부터 적용돼요. 파일 하나에는 처음 맞는 규칙 하나만 적용돼요.', 'Rules apply from the top. Each file gets only the first rule it matches.')}</p>
    <button id="add-rule" class="primary small">${L('+ 직접 만들기', '+ New rule')}</button>
  </div>`;
  const d = state.draft;
  const cards = t.rules.map((r, i) => (d && d.id === r.id ? editorHtml() : ruleCard(r, i, t.rules.length))).join('');
  $('#rules').innerHTML =
    (d && !d.id ? editorHtml() : '') +
    (t.rules.length ? `<div class="rule-list">${cards}</div>` : d ? '' : `<div class="block"><div class="empty">${L('아직 규칙이 없어요. 말로 만들거나 직접 만들어 보세요.', 'No rules yet. Describe one or make one yourself.')}</div></div>`);
  if (d) refreshMatches();
}

function openEditor(rule) {
  state.draft = newDraft(rule);
  renderRules();
  $('#editor')?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  $('#d-name')?.focus();
}

function addTag(kind, raw) {
  const d = state.draft;
  const list = kind === 'ext' ? d.exts : d.words;
  for (let v of raw.split(kind === 'ext' ? /[,\s]+/ : /,+/)) {
    v = kind === 'ext' ? v.trim().replace(/^\*?\./, '').toLowerCase() : v.trim();
    if (v && !list.includes(v)) list.push(v);
  }
}

async function saveDraft() {
  const d = state.draft;
  const t = currentTarget();
  if ($('#d-ext')?.value.trim()) addTag('ext', $('#d-ext').value);
  if ($('#d-word')?.value.trim()) addTag('word', $('#d-word').value);
  const rule = draftRule(d);
  if (!rule.match.extensions.length && !rule.match.nameIncludes.length && !rule.match.olderThanDays) {
    toast(L('1단계에서 파일 종류나 이름 단어, 기간 중 하나는 골라 주세요.', 'In step 1, pick a file kind, a word in the name or an age.'));
    return;
  }
  if (rule.action === 'move' && !rule.dest) {
    toast(L('2단계에서 옮길 폴더를 골라 주세요.', 'In step 2, pick a folder to move them to.'));
    return;
  }
  if (d.id) {
    const r = t.rules.find((x) => x.id === d.id);
    Object.assign(r, rule, { enabled: r.enabled });
  } else {
    t.rules.unshift({ id: `r_${Date.now().toString(36)}`, ...rule });
  }
  state.draft = null;
  await saveConfig();
  renderRules();
  await doScan();
  toast(L('규칙을 저장했어요. 맨 위에 넣어서 먼저 적용돼요.', 'Rule saved at the top, so it applies first.'));
}

async function makeRuleFromText() {
  const input = $('#nl-text');
  const text = input.value.trim();
  if (!text) return input.focus();
  const btn = $('#nl-make');
  btn.disabled = true;
  btn.innerHTML = `<span class="spinner"></span>${L('만드는 중…', 'Making…')}`;
  try {
    const rule = await tidy.ruleFromText(state.targetId, text);
    state.draft = { ...newDraft(rule), id: null };
    renderRules();
    $('#editor')?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    toast(L('Claude가 규칙을 채웠어요. 확인하고 저장하세요.', 'Claude filled in the rule. Check it and save.'));
  } catch (err) {
    toast(L(`규칙 만들기 실패: ${err.message}`, `Couldn't make the rule: ${err.message}`), 6000);
  } finally {
    if ($('#nl-make')) {
      $('#nl-make').disabled = false;
      $('#nl-make').textContent = L('Claude로 만들기', 'Make it with Claude');
    }
  }
}

async function pickDest() {
  const t = currentTarget();
  const picked = await tidy.pickFolder(state.draft.dest ? expand(state.draft.dest) : expand(t.path));
  if (!picked) return;
  const root = expand(t.path);
  state.draft.dest = picked.startsWith(root + state.sep) ? picked.slice(root.length + 1).split(state.sep).join('/') : contract(picked);
  renderEditor();
}

async function loadFolders() {
  try {
    state.folders = await tidy.folders();
  } catch {
    state.folders = [];
  }
  $('#folder-list').innerHTML = state.folders.map((f) => `<option value="${esc(f)}"></option>`).join('');
}

// ---------- 기록 ----------

async function renderHistory() {
  const runs = await tidy.history();
  $('#undo').disabled = !runs.some((r) => !r.undone);
  const who = { 'claude-mcp': '<span class="tag ai">Claude</span>', auto: `<span class="tag auto">${L('자동', 'Auto')}</span>` };
  $('#history').innerHTML = runs.length
    ? `<div class="block">${runs
        .map((r) => {
          const moved = r.ops.filter((o) => o.type === 'move').length;
          const trashed = r.ops.length - moved;
          const when = new Date(r.at).toLocaleString(L('ko-KR', 'en-US'), { dateStyle: 'medium', timeStyle: 'short' });
          return `<details class="run">
          <summary><span>${who[r.source] || `<span class="tag">${L('앱', 'App')}</span>`} ${esc(when)} · ${L('옮김', 'moved')} ${moved}${trashed ? ` · ${L('휴지통', 'Trash')} ${trashed}` : ''}</span>${
            r.undone ? `<span class="tag warn">${L('되돌림', 'Undone')}</span>` : ''
          }</summary>
          <div class="run-ops">${r.ops
            .map((o) => `<div title="${esc(o.fromLabel)}">${o.type === 'move' ? `${esc(o.fromLabel)} → ${esc(o.toLabel)}` : `${L('휴지통', 'Trash')} ← ${esc(o.fromLabel)}`}</div>`)
            .join('')}</div>
        </details>`;
        })
        .join('')}</div>`
    : `<div class="block"><div class="empty">${L('아직 정리한 기록이 없어요.', 'Nothing tidied yet.')}</div></div>`;
}

async function doUndo() {
  try {
    const res = await tidy.undo();
    if (!res.ok) return toast(res.message);
    const extra = res.trashed.length
      ? L(` 휴지통으로 보낸 ${res.trashed.length}개는 휴지통에서 직접 꺼내 주세요.`, ` Take the ${res.trashed.length} sent to the Trash back out yourself.`)
      : '';
    const fail = res.failed.length
      ? L(` ${res.failed.length}개는 되돌리지 못했어요(${res.failed[0].reason}).`, ` ${res.failed.length} couldn't be undone (${res.failed[0].reason}).`)
      : '';
    toast(L(`${res.restored}개 파일을 원래 자리로 돌려놨어요.`, `Put ${res.restored} files back where they were.`) + extra + fail, 6000);
  } catch (err) {
    toast(L(`되돌리기 실패: ${err.message}`, `Undo failed: ${err.message}`), 6000);
  }
  await renderHistory();
  await doScan();
}

// ---------- 자동 정리 · Claude ----------

function setPill() {
  const s = state.integration;
  if (!s) return;
  const linked = s.claudeCode || s.claudeDesktop;
  $('#claude-dot').className = `dot ${cliReady() || linked ? 'on' : 'off'}`;
  $('#claude-pill-text').textContent = linked ? L('Claude와 연결됨', 'Connected to Claude') : cliReady() ? L('Claude CLI 사용 가능', 'Claude CLI available') : L('Claude 연결 안 됨', 'Claude not connected');
}

async function refreshIntegration() {
  try {
    state.integration = await tidy.integrationStatus();
  } catch (err) {
    state.integration = { cli: { installed: false, error: err.message }, locations: [], claudeCode: false, claudeDesktop: false, launch: null };
  }
  setPill();
}

const switchHtml = (attr, on) => `<label class="switch"><input type="checkbox" ${attr} ${on ? 'checked' : ''} /><span></span></label>`;
const dotLine = (on, onText, offText) => `<div class="status-line"><span class="dot ${on ? 'on' : 'off'}"></span>${on ? onText : offText}</div>`;

function renderSettings() {
  const cfg = state.config;
  const st = cfg.settings;
  const delays = [
    [5, L('5초', '5 sec')],
    [10, L('10초', '10 sec')],
    [30, L('30초', '30 sec')],
    [60, L('1분', '1 min')],
    [300, L('5분', '5 min')],
  ];
  const autoCard = `<div class="card">
    <h3>${L('자동 정리', 'Auto-tidy')}</h3>
    <p>${L(
      `켜 둔 폴더에 새 파일이 들어오면, 다 받아질 때까지 기다렸다가 규칙대로 정리해요. 창을 닫아도 ${
        state.platform === 'darwin' ? '메뉴 막대' : '작업 표시줄 알림 영역'
      }에서 계속 지켜봐요. AI 제안은 쓰지 않고 내가 만든 규칙만 써요.`,
      `When a new file lands in a folder you turned on, Smash! waits until it finishes downloading, then sorts it by your rules. It keeps watching from the ${
        state.platform === 'darwin' ? 'menu bar' : 'notification area'
      } after you close the window. Only your own rules are used, never AI picks.`,
    )}</p>
    <div class="settings-list">
      ${cfg.targets
        .map(
          (t) => `<div class="setting-row"><div><b>${esc(targetName(t))}</b><div class="muted">${esc(t.path)} · ${L(`켜진 규칙 ${t.rules.filter((r) => r.enabled).length}개`, `${t.rules.filter((r) => r.enabled).length} rules on`)}</div></div>${switchHtml(
            `data-auto="${esc(t.id)}"`,
            t.auto,
          )}</div>`,
        )
        .join('')}
      <div class="setting-row"><div><b>${L('기다리는 시간', 'Wait time')}</b><div class="muted">${L('파일이 이만큼 그대로면 다 받아진 걸로 봐요', 'A file that stays the same this long counts as downloaded')}</div></div>
        <select id="auto-delay">${delays.map(([v, l]) => `<option value="${v}" ${st.autoDelaySec === v ? 'selected' : ''}>${l}</option>`).join('')}</select></div>
      <div class="setting-row"><div><b>${L('정리할 때 등짝 스매시 효과', 'Smash effect when tidying')}</b><div class="muted">${L('정리하기를 누르면 손바닥이 "찰싹!!" 내리쳐요. 자동 정리 때는 안 나와요', 'A palm comes down with a "SMACK!!" when you press tidy. Not shown for auto-tidy')}</div></div>${switchHtml('id="set-smash"', st.smash)}</div>
      <div class="setting-row"><div><b>${L('정리한 뒤 폴더 열기', 'Open the folder afterwards')}</b><div class="muted">${L(
        `정리하기를 누르면 파일이 들어간 폴더를 ${state.platform === 'darwin' ? 'Finder' : '탐색기'}로 열어 줘요`,
        `Opens the folder the files went to in ${state.platform === 'darwin' ? 'Finder' : 'Explorer'}`,
      )}</div></div>${switchHtml('id="set-open"', st.openAfter)}</div>
      <div class="setting-row"><div><b>${L('정리하면 알림 보내기', 'Notify me after tidying')}</b></div>${switchHtml('id="set-notify"', st.notify)}</div>
      <div class="setting-row"><div><b>${L('컴퓨터 켤 때 같이 실행', 'Open at login')}</b><div class="muted">${L('창 없이 조용히 시작해요 (설치한 앱에서만 동작)', 'Starts quietly without a window (installed app only)')}</div></div>${switchHtml('id="set-login"', st.openAtLogin)}</div>
      <div class="setting-row"><div><b>언어 · Language</b><div class="muted">${L('바꾸면 화면을 다시 불러와요', 'The window reloads when you change it')}</div></div>
        <select id="set-lang">${[
          ['system', L('시스템 설정 따르기', 'Same as system')],
          ['ko', '한국어'],
          ['en', 'English'],
        ]
          .map(([v, l]) => `<option value="${v}" ${(st.language || 'system') === v ? 'selected' : ''}>${l}</option>`)
          .join('')}</select></div>
    </div>
  </div>`;

  const s = state.integration;
  if (!s) {
    $('#settings').innerHTML = `${autoCard}<div class="card"><span class="spinner"></span>${L('Claude 연결 확인 중…', 'Checking Claude…')}</div>`;
    return;
  }
  const cli = s.cli;
  const model = cfg.ai?.model || 'sonnet';
  const shellQuote = (v) => (/[\s"'$`\\]/.test(v) ? `"${v.replace(/(["\\$`])/g, '\\$1')}"` : v);
  const cmd = s.launch
    ? `claude mcp add -s user ${s.mcpName} -e ELECTRON_RUN_AS_NODE=1 -- ${shellQuote(s.launch.command)} ${s.launch.args.map(shellQuote).join(' ')}`
    : '';
  $('#settings').innerHTML = `${autoCard}
    <div class="card">
      <h3>${L('Claude 연결', 'Claude')} <span class="muted">${L('API 키 없이', 'no API key')}</span></h3>
      <p>${L('내 PC에 로그인된 Claude를 그대로 빌려 써요. 사용량은 내 Claude 구독에 포함돼요.', 'Uses the Claude you are signed in to on this computer. Usage counts toward your Claude plan.')}</p>
      ${
        cli.installed
          ? dotLine(!cli.error, `Claude Code CLI ${esc(cli.version || '')} <span class="muted">${esc(cli.path)}</span>`, L(`CLI 실행 오류: ${esc(cli.error)}`, `CLI error: ${esc(cli.error)}`))
          : dotLine(false, '', L('Claude Code CLI가 없어요', 'Claude Code CLI not found'))
      }
      <div class="card-actions">
        ${cli.installed ? '' : `<button class="primary small" data-url="https://claude.com/claude-code">${L('설치 방법 보기', 'How to install')}</button>`}
        <label class="muted">${L('자리 제안·말로 규칙 만들기에 쓸 모델', 'Model for suggestions and described rules')}
          <select id="ai-model">${['haiku', 'sonnet', 'opus'].map((m) => `<option value="${m}" ${m === model ? 'selected' : ''}>${m}</option>`).join('')}</select>
        </label>
      </div>
    </div>

    <div class="card">
      <h3>${L('Claude Code에 MCP 서버로 등록', 'Add as an MCP server to Claude Code')}</h3>
      <p>${L(
        '등록하면 Claude Code에서 “다운로드 정리해줘”라고 말할 때 이 앱의 규칙·기록을 그대로 써요. 찾은 Claude 설정 위치 모두에 등록해요.',
        'Then when you ask Claude Code to “tidy my Downloads”, it uses this app’s rules and history. Added to every Claude config location found.',
      )}</p>
      <div class="loc-list">${(s.locations || [])
        .map((l) => `<div class="loc-row"><span class="dot ${l.registered ? 'on' : 'off'}"></span><span>${esc(l.label)}</span><span class="muted">${l.registered ? L('등록됨', 'Added') : L('등록 안 됨', 'Not added')}</span></div>`)
        .join('')}</div>
      <div class="card-actions">
        <button class="primary small" id="reg-code" ${cliReady() ? '' : 'disabled'}>${s.claudeCode ? L('다시 등록', 'Add again') : L('모두 등록하기', 'Add to all')}</button>
        <span class="muted">${L('등록 후 새로 연 Claude Code 세션부터 보여요.', 'Shows up in Claude Code sessions opened after this.')}</span>
      </div>
      <details class="cmd-details"><summary>${L('직접 등록하는 명령 보기', 'Show the command to add it yourself')}</summary><pre class="cmd">${esc(cmd)}</pre></details>
    </div>

    <div class="card">
      <h3>${L('Claude Desktop에 MCP 서버로 등록', 'Add as an MCP server to Claude Desktop')}</h3>
      <p>${L(
        `Claude Desktop 설정 파일(${esc(s.desktopConfig)})에 추가해요. 기존 파일은 .bak으로 백업하고, 추가한 뒤 Claude Desktop을 다시 시작해야 해요.`,
        `Adds it to the Claude Desktop config (${esc(s.desktopConfig)}). The old file is backed up as .bak. Restart Claude Desktop afterwards.`,
      )}</p>
      ${dotLine(s.claudeDesktop, L('등록됨', 'Added'), s.desktopInstalled ? L('등록 안 됨', 'Not added') : L('Claude Desktop이 설치돼 있지 않아요', 'Claude Desktop is not installed'))}
      <div class="card-actions"><button class="primary small" id="reg-desktop" ${s.desktopInstalled ? '' : 'disabled'}>${s.claudeDesktop ? L('다시 등록', 'Add again') : L('등록하기', 'Add')}</button></div>
    </div>
    <p class="hint">${L('설정 파일', 'Settings file')}: ${esc(state.configPath)}</p>`;
}

async function register(kind) {
  const btn = kind === 'code' ? $('#reg-code') : $('#reg-desktop');
  btn.disabled = true;
  btn.innerHTML = `<span class="spinner"></span>${L('등록 중…', 'Adding…')}`;
  try {
    if (kind === 'code') {
      const results = await tidy.registerClaudeCode();
      const failed = results.filter((r) => !r.ok);
      toast(
        failed.length
          ? L(`${results.length - failed.length}곳 등록, ${failed.length}곳 실패`, `Added to ${results.length - failed.length}, ${failed.length} failed`) +
            `: ${failed[0].location} — ${failed[0].error || L('확인 실패', 'check failed')}`
          : L(`${results.length}곳에 등록했어요. 새 Claude Code 세션부터 쓸 수 있어요.`, `Added to ${results.length} places. New Claude Code sessions can use it.`),
        6000,
      );
    } else {
      const file = await tidy.registerClaudeDesktop();
      toast(L(`${file}에 추가했어요. Claude Desktop을 다시 시작해 주세요.`, `Added to ${file}. Restart Claude Desktop.`), 5000);
    }
  } catch (err) {
    toast(L(`등록 실패: ${err.message}`, `Couldn't add it: ${err.message}`), 6000);
  }
  await refreshIntegration();
  renderSettings();
}

// ---------- 이벤트 ----------

function handleDraftClick(el) {
  const d = state.draft;
  const t = currentTarget();
  if (el.dataset.cat) {
    const cat = state.categories.find((c) => c.key === el.dataset.cat);
    const all = cat.extensions.every((x) => d.exts.includes(x));
    d.exts = all ? d.exts.filter((x) => !cat.extensions.includes(x)) : [...new Set([...d.exts, ...cat.extensions])];
    if (!all) {
      if (!d.name.trim() || state.categories.some((c) => c.label === d.name)) d.name = cat.label;
      if (cat.key === 'installer') d.action = 'trash';
      else if (!d.dest && cat.folder) d.dest = expand(t.path) === expand('~/Documents') ? cat.folder : `~/Documents/${cat.folder}`;
    }
    renderEditor();
    return true;
  }
  if (el.dataset.untag) {
    const list = el.dataset.untag === 'ext' ? 'exts' : 'words';
    d[list] = d[list].filter((v) => v !== el.dataset.value);
    renderEditor();
    return true;
  }
  if (el.dataset.days) {
    d.days = Number(el.dataset.days);
    renderEditor();
    return true;
  }
  if (el.dataset.action) {
    d.action = el.dataset.action;
    renderEditor();
    return true;
  }
  return false;
}

document.addEventListener('click', async (e) => {
  const el = e.target.closest('button, [data-tab]');
  if (!el) return;
  if (el.dataset.tab) return switchTab(el.dataset.tab);
  if (el.dataset.goto) return switchTab(el.dataset.goto);
  if (el.dataset.url) return tidy.openUrl(el.dataset.url);
  if (el.classList.contains('target-btn')) return selectTarget(el.dataset.id);
  if (el.dataset.collapse) {
    const key = el.dataset.collapse;
    if (state.collapsed.has(key)) state.collapsed.delete(key);
    else state.collapsed.add(key);
    return renderPreview();
  }
  if (state.draft && handleDraftClick(el)) return;
  const t = currentTarget();
  if (el.dataset.ruleEdit) return openEditor(t.rules.find((r) => r.id === el.dataset.ruleEdit));
  if (el.dataset.ruleDelete) {
    t.rules = t.rules.filter((r) => r.id !== el.dataset.ruleDelete);
    await saveConfig();
    renderRules();
    return doScan();
  }
  if (el.dataset.ruleMove) {
    const i = t.rules.findIndex((r) => r.id === el.dataset.ruleMove);
    const j = i + Number(el.dataset.dir);
    [t.rules[i], t.rules[j]] = [t.rules[j], t.rules[i]];
    await saveConfig();
    renderRules();
    return doScan();
  }
  switch (el.id) {
    case 'scan': return doScan();
    case 'apply': return doApply();
    case 'ai-suggest': return aiSuggest();
    case 'toggle-all': {
      if (state.unchecked.size) state.unchecked.clear();
      else allPlanItems().forEach((it) => state.unchecked.add(it.id));
      return renderPreview();
    }
    case 'add-target': return addTarget();
    case 'remove-target': return removeTarget();
    case 'add-rule': return openEditor(null);
    case 'nl-make': return makeRuleFromText();
    case 'cancel-rule':
      state.draft = null;
      return renderRules();
    case 'save-rule': return saveDraft();
    case 'pick-dest': return pickDest();
    case 'undo': return doUndo();
    case 'reg-code': return register('code');
    case 'reg-desktop': return register('desktop');
    default:
  }
});

// 태그 상자 빈 곳을 누르면 입력칸으로
document.addEventListener('mousedown', (e) => {
  const box = e.target.closest('.tag-box');
  if (box && e.target === box) {
    e.preventDefault();
    box.querySelector('input')?.focus();
  }
});

document.addEventListener('keydown', (e) => {
  const el = e.target;
  if (e.isComposing) return;
  if (el.id === 'nl-text' && e.key === 'Enter') return makeRuleFromText();
  if (el.id === 'd-ext' || el.id === 'd-word') {
    const kind = el.id === 'd-ext' ? 'ext' : 'word';
    if ((e.key === 'Enter' || e.key === ',') && el.value.trim()) {
      e.preventDefault();
      addTag(kind, el.value);
      return renderEditor(el.id);
    }
    if (e.key === 'Backspace' && !el.value) {
      const list = kind === 'ext' ? state.draft.exts : state.draft.words;
      if (list.length) {
        list.pop();
        return renderEditor(el.id);
      }
    }
  }
});

document.addEventListener('input', (e) => {
  const d = state.draft;
  if (!d) return;
  if (e.target.id === 'd-name') d.name = e.target.value;
  if (e.target.id === 'd-dest') {
    d.dest = e.target.value;
    $('#dest-preview').innerHTML = destPreviewHtml(d);
  }
});

document.addEventListener('change', async (e) => {
  const el = e.target;
  if (el.dataset.item) {
    if (el.checked) state.unchecked.delete(el.dataset.item);
    else state.unchecked.add(el.dataset.item);
    el.closest('.trow').classList.toggle('off', !el.checked);
    syncFolderChecks();
    return updateApplyButton();
  }
  if (el.dataset.folder) {
    const ids = state.folderItems.get(el.dataset.folder) || [];
    ids.forEach((id) => (el.checked ? state.unchecked.delete(id) : state.unchecked.add(id)));
    return renderPreview();
  }
  if (el.dataset.ruleToggle) {
    const r = currentTarget().rules.find((x) => x.id === el.dataset.ruleToggle);
    r.enabled = el.checked;
    await saveConfig();
    renderRules();
    return doScan();
  }
  if (el.dataset.auto) {
    const t = state.config.targets.find((x) => x.id === el.dataset.auto);
    t.auto = el.checked;
    await saveConfig();
    renderSettings();
    return toast(
      el.checked
        ? L(`${targetName(t)} 자동 정리를 켰어요. 짝! 하고 지켜볼게요.`, `Auto-tidy on for ${targetName(t)}. I'm watching.`)
        : L(`${targetName(t)} 자동 정리를 껐어요.`, `Auto-tidy off for ${targetName(t)}.`),
    );
  }
  if (el.id === 'auto-delay') {
    state.config.settings.autoDelaySec = Number(el.value);
    return saveConfig();
  }
  if (['set-notify', 'set-login', 'set-smash', 'set-open'].includes(el.id)) {
    state.config.settings[{ 'set-notify': 'notify', 'set-login': 'openAtLogin', 'set-smash': 'smash', 'set-open': 'openAfter' }[el.id]] = el.checked;
    return saveConfig();
  }
  if (el.id === 'set-lang') {
    state.config.settings.language = el.value;
    await saveConfig();
    sessionStorage.setItem('tab', 'settings');
    return location.reload();
  }
  if (el.id === 'ai-model') {
    state.config.ai = { ...(state.config.ai || {}), model: el.value };
    await saveConfig();
    toast(L(`Claude 작업에 ${el.value} 모델을 써요.`, `Using ${el.value} for Claude tasks.`));
  }
});

tidy.onAutoRan(async ({ count }) => {
  if (count) toast(L(`짝! 자동으로 ${count}개 치웠어요.`, `Smack! Auto-tidied ${count} files.`));
  if (!state.busy && !state.aiItems.length) await doScan();
  if ($('#tab-history').classList.contains('active')) renderHistory();
});
tidy.onTab((tab) => switchTab(tab));
tidy.onConfigChanged(async () => {
  const { config } = await tidy.getConfig();
  state.config = config;
  renderTargets();
  if ($('#tab-settings').classList.contains('active')) renderSettings();
});

// index.html에 적힌 고정 문구(한국어)를 화면 언어에 맞춘다.
function translateStatic() {
  document.documentElement.lang = LANG;
  if (LANG === 'ko') return;
  const set = (sel, text) => {
    const el = $(sel);
    if (el) el.textContent = text;
  };
  set('.brand-sub', 'Clean your room!');
  set('.side-label', 'Folders to tidy');
  set('#add-target', '+ Add folder');
  set('#claude-pill-text', 'Checking Claude…');
  set('#scan', 'Refresh');
  set('#apply', 'Smack! Tidy up');
  set('.tab[data-tab="preview"]', 'Preview');
  set('.tab[data-tab="rules"]', 'Rules');
  set('.tab[data-tab="history"]', 'History');
  set('.tab[data-tab="settings"]', 'Auto-tidy · Claude');
  set('#remove-target', 'Remove this folder from the list');
  set('#tab-history .hint', 'Moved files can be undone. Files sent to the Trash have to be taken out of the Trash yourself.');
  set('#undo', 'Undo last run');
}

(async function init() {
  const { config, configPath, home, sep, categories, platform, lang } = await tidy.getConfig();
  Object.assign(state, { config, configPath, home, sep, categories, platform });
  LANG = lang;
  translateStatic();
  loadFolders();
  refreshIntegration().then(() => {
    if (state.scan) renderPreview();
    if ($('#tab-settings').classList.contains('active')) renderSettings();
    if ($('#tab-rules').classList.contains('active') && !state.draft) renderRules();
  });
  await selectTarget(config.targets[0]?.id || null);
  // 언어를 바꿔 다시 불러왔으면 보던 설정 탭으로 돌아간다.
  const tab = sessionStorage.getItem('tab');
  sessionStorage.removeItem('tab');
  if (tab) switchTab(tab);
})();
