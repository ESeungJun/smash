'use strict';
/* global tidy */

const $ = (sel) => document.querySelector(sel);
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
        <span>${esc(targetName(t))}</span>${t.auto ? '<span class="auto-badge" title="자동 정리 켜짐">자동</span>' : ''}
      </button>`,
    )
    .join('');
}

async function selectTarget(id) {
  state.targetId = id;
  state.draft = null;
  renderTargets();
  const t = currentTarget();
  $('#target-path').textContent = t ? targetName(t) : '폴더를 추가해 주세요';
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
  toast('폴더를 추가했어요. 규칙 탭에서 정리 규칙을 만들어 보세요.');
}

let removeArmed = false;
async function removeTarget() {
  const btn = $('#remove-target');
  if (!removeArmed) {
    removeArmed = true;
    btn.textContent = '한 번 더 누르면 목록에서 뺍니다 (파일은 그대로)';
    setTimeout(() => {
      removeArmed = false;
      btn.textContent = '이 폴더를 목록에서 빼기';
    }, 3000);
    return;
  }
  removeArmed = false;
  btn.textContent = '이 폴더를 목록에서 빼기';
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
  $('#plan').innerHTML = '<div class="block"><div class="empty"><span class="spinner"></span>살펴보는 중…</div></div>';
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
  const isAi = String(it.reason || '').startsWith('AI');
  const fromDir = baseName(dirOf(it.src));
  const sameDir = it.action !== 'trash' && dirOf(it.src) === dirOf(it.dest);
  const tags = [
    isAi ? '<span class="tag ai">AI 제안</span>' : '',
    it.note === 'duplicate' ? '<span class="tag warn">중복</span>' : '',
    it.note === 'renamed' ? `<span class="tag warn" title="같은 이름이 있어 ${esc(baseName(it.dest))}(으)로 저장">이름 변경</span>` : '',
  ].join('');
  const where =
    it.note === 'duplicate'
      ? `같은 파일이 ${esc(destText(dirOf(it.duplicateOf)))}에 있어요`
      : `${sameDir ? '' : `${esc(fromDir)}에서 · `}${esc(String(it.reason || '').replace(/^규칙: /, ''))}`;
  return `<li class="trow file ${off ? 'off' : ''}" data-depth="${Math.min(depth, 8)}">
    <span class="caret-space"></span>
    <input type="checkbox" data-item="${esc(it.id)}" ${off ? '' : 'checked'} aria-label="${esc(it.name)} 포함" />
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
      <button class="caret ${collapsed ? 'closed' : ''}" data-collapse="${esc(node.abs)}" aria-label="펼치기/접기">▾</button>
      <input type="checkbox" data-folder="${esc(node.abs)}" aria-label="${esc(node.name)} 전체" />
      <span class="ficon">${ico(icon)}</span>
      <span class="tname folder-name">${esc(node.name)}</span>
      <span class="tmeta">${ids.length}개</span>
      <span class="tags">${fresh ? '<span class="tag new">새 폴더</span>' : ''}</span>
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
  $('#target-sub').textContent = `${scan.rootLabel} · 옮길 파일 ${moving}개${trash.length ? ` · 휴지통 ${trash.length}개` : ''}${
    newDirs.size ? ` · 새 폴더 ${newDirs.size}개` : ''
  }${unmatched.length ? ` · 규칙 밖 ${unmatched.length}개` : ''}`;

  state.folderItems = new Map();
  const trashNode = { name: '휴지통', abs: '::trash', children: new Map(), items: trash };
  const tree = [
    ...tops.map((n) => folderRow(n, 0, newDirs)),
    trash.length ? folderRow(trashNode, 0, newDirs, { icon: 'trash', isNew: false }) : '',
  ].join('');

  $('#plan').innerHTML = `<div class="block">
    <div class="block-head">
      <div>
        <div class="block-title">정리하면 이렇게 돼요<span class="count">${items.length}</span></div>
        <div class="block-sub">폴더 옆 ▾를 누르면 접고 펼 수 있어요. 체크를 풀면 그 파일은 그대로 둬요.</div>
      </div>
      ${items.length ? '<button class="link" id="toggle-all">전체 선택/해제</button>' : ''}
    </div>
    ${
      items.length
        ? `<ul class="tree">${tree}</ul>`
        : '<div class="empty big"><b>치울 게 없어요.</b><br />규칙에 맞는 파일이 하나도 없어요. 엄마가 칭찬하겠네요.</div>'
    }
  </div>`;
  syncFolderChecks();

  const aiButton = !unmatched.length
    ? ''
    : state.integration && !cliReady()
      ? '<button class="ghost small" data-goto="settings">Claude 연결하기</button>'
      : '<button class="ghost small" id="ai-suggest">Claude에게 자리 물어보기</button>';
  $('#unmatched').innerHTML = unmatched.length
    ? `<div class="block">
        <div class="block-head">
          <div>
            <div class="block-title">규칙에 안 걸린 파일<span class="count">${unmatched.length}</span></div>
            <div class="block-sub">Claude에게는 파일 이름·크기·날짜와 폴더 목록만 보내요. 파일 내용은 안 보내요.</div>
          </div>
          ${aiButton}
        </div>
        <ul class="rows">${unmatched
          .map(
            (f) => `<li class="plain-row"><span class="ficon">${ico('file')}</span><span class="fname" title="${esc(f.srcLabel)}">${esc(f.name)}</span><span class="dest">${fmtSize(f.size)}</span></li>`,
          )
          .join('')}</ul>
        ${state.aiNoSuggestion.length ? `<div class="note">Claude가 확신하지 못한 ${state.aiNoSuggestion.length}개는 그대로 둬요.</div>` : ''}
      </div>`
    : '';
  updateApplyButton();
}

function updateApplyButton() {
  const n = checkedItems().length;
  const btn = $('#apply');
  btn.disabled = state.busy || n === 0;
  btn.textContent = n ? `짝! 정리하기 (${n})` : '짝! 정리하기';
}

async function aiSuggest() {
  const btn = $('#ai-suggest');
  const aiNames = new Set(state.aiItems.map((i) => i.name));
  const unmatched = state.scan.unmatched.filter((f) => !aiNames.has(f.name));
  btn.disabled = true;
  btn.innerHTML = '<span class="spinner"></span>Claude가 보는 중…';
  try {
    const res = await tidy.suggest(state.targetId, unmatched);
    state.aiItems.push(...res.items);
    state.aiNoSuggestion = res.noSuggestion;
    renderPreview();
    toast(res.items.length ? `Claude가 ${res.items.length}개 파일의 자리를 골랐어요. 트리에서 확인하세요.` : 'Claude도 확신할 만한 자리를 못 찾았어요.');
  } catch (err) {
    btn.disabled = false;
    btn.textContent = 'Claude에게 자리 물어보기';
    toast(`제안 실패: ${err.message}`, 6000);
  }
}

async function doApply() {
  const items = checkedItems();
  if (!items.length) return;
  state.busy = true;
  updateApplyButton();
  $('#apply').innerHTML = '<span class="spinner"></span>정리 중…';
  try {
    const { results } = await tidy.apply(items);
    const ok = results.filter((r) => r.ok).length;
    const failed = results.filter((r) => !r.ok);
    toast(
      failed.length ? `${ok}개 정리, ${failed.length}개 실패: ${failed[0].name} — ${failed[0].message}` : `짝! ${ok}개 치웠어요. 기록 탭에서 되돌릴 수 있어요.`,
      failed.length ? 6000 : 3500,
    );
  } catch (err) {
    toast(`실행 실패: ${err.message}`, 6000);
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
  if (r.match.olderThanDays) parts.push(`<span class="chip soft">${r.match.olderThanDays}일 지난</span>`);
  if (r.match.nameIncludes.length) parts.push(`이름에 ${chipList(r.match.nameIncludes.map((w) => `"${w}"`), 3)} 가 들어간`);
  if (r.match.extensions.length) parts.push(chipList(r.match.extensions));
  const what = parts.length ? `${parts.join(' ')} 파일은` : '<span class="warn-text">조건이 없어 적용되지 않아요</span>';
  const how = r.action === 'trash' ? '<b class="to trash">휴지통</b>으로 보내요' : `<b class="to">${esc(destText(r.dest))}</b> 로 옮겨요`;
  return `${what} → ${how}`;
}

function ruleCard(r, i, total) {
  return `<div class="rule-card ${r.enabled ? '' : 'disabled'}">
    <div class="rule-body">
      <div class="rule-name">${esc(r.name)}</div>
      <div class="rule-sentence">${ruleSentence(r)}</div>
    </div>
    <div class="rule-side">
      <label class="switch" title="켜기/끄기"><input type="checkbox" data-rule-toggle="${esc(r.id)}" ${r.enabled ? 'checked' : ''} /><span></span></label>
      <div class="rule-actions">
        <button class="icon-btn" data-rule-move="${esc(r.id)}" data-dir="-1" ${i === 0 ? 'disabled' : ''} title="먼저 적용">↑</button>
        <button class="icon-btn" data-rule-move="${esc(r.id)}" data-dir="1" ${i === total - 1 ? 'disabled' : ''} title="나중에 적용">↓</button>
        <button class="icon-btn" data-rule-edit="${esc(r.id)}">수정</button>
        <button class="icon-btn danger" data-rule-delete="${esc(r.id)}">삭제</button>
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
  name: d.name.trim() || '새 규칙',
  enabled: true,
  match: { extensions: d.exts, nameIncludes: d.words, olderThanDays: d.days },
  action: d.action,
  dest: d.action === 'trash' ? '' : d.dest.trim(),
});

const DAY_OPTIONS = [
  [0, '바로'],
  [1, '하루 지나면'],
  [7, '일주일 지나면'],
  [30, '한 달 지나면'],
  [90, '석 달 지나면'],
];

function matchLineHtml(d) {
  const t = currentTarget();
  const m = d.matches;
  const body =
    m == null
      ? '<span class="spinner"></span>확인 중…'
      : m.length
        ? `지금 <b>${esc(targetName(t))}</b>에서 맞는 파일 <b>${m.length}개</b>: ${m.slice(0, 6).map(esc).join(', ')}${m.length > 6 ? ' …' : ''}`
        : `지금 <b>${esc(targetName(t))}</b>에는 맞는 파일이 없어요.`;
  return `<div class="match-line" id="match-line">${body}</div>`;
}

function destPreviewHtml(d) {
  if (!d.dest) return '옮길 폴더를 골라 주세요.';
  const label = destText(d.dest);
  const exists = state.folders.some((f) => destText(f) === label);
  return `→ <b>${esc(label)}</b>${exists ? '' : ' <span class="tag new">새 폴더로 만들어요</span>'}`;
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
    list.map((v) => `<span class="tag-chip">${esc(v)}<button type="button" data-untag="${kind}" data-value="${esc(v)}" aria-label="${esc(v)} 빼기">×</button></span>`).join('');
  return `<div class="editor" id="editor">
    <div class="editor-head">
      <input class="name-input" id="d-name" type="text" placeholder="규칙 이름 (예: 회사 서류)" value="${esc(d.name)}" />
    </div>

    <div class="step">
      <div class="step-title"><span class="step-no">1</span>어떤 파일을 치울까요?</div>
      <div class="cats">${catChips}</div>
      <div class="field">
        <div class="field-label">확장자</div>
        <div class="tag-box">${tags(d.exts, 'ext')}<input id="d-ext" type="text" placeholder="${d.exts.length ? '추가…' : '예: pdf, hwp (엔터)'}" /></div>
      </div>
      <div class="field">
        <div class="field-label">이름에 이 단어가 있으면 <span class="muted">(선택)</span></div>
        <div class="tag-box">${tags(d.words, 'word')}<input id="d-word" type="text" placeholder="예: 계약서, 영수증 (엔터)" /></div>
      </div>
      <div class="field">
        <div class="field-label">언제?</div>
        <div class="seg">${DAY_OPTIONS.map(([v, l]) => `<button type="button" data-days="${v}" class="${d.days === v ? 'on' : ''}">${l}</button>`).join('')}</div>
      </div>
    </div>

    <div class="step">
      <div class="step-title"><span class="step-no">2</span>어떻게 할까요?</div>
      <div class="seg wide">
        <button type="button" data-action="move" class="${d.action === 'move' ? 'on' : ''}">폴더로 옮기기</button>
        <button type="button" data-action="trash" class="${d.action === 'trash' ? 'on' : ''}">휴지통으로 보내기</button>
      </div>
      ${
        d.action === 'move'
          ? `<div class="field">
              <div class="inline">
                <input id="d-dest" type="text" list="folder-list" placeholder="폴더 이름이나 경로 (예: ~/Documents/회사)" value="${esc(d.dest)}" />
                <button type="button" class="ghost small" id="pick-dest">폴더 고르기…</button>
              </div>
              <div class="dest-preview" id="dest-preview">${destPreviewHtml(d)}</div>
            </div>`
          : '<p class="hint">휴지통으로 보내요. 영구 삭제는 하지 않아요.</p>'
      }
    </div>

    ${matchLineHtml(d)}
    <div class="editor-actions">
      <button type="button" class="ghost" id="cancel-rule">취소</button>
      <button type="button" class="primary" id="save-rule">저장</button>
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
    <div class="nl-title">말로 규칙 만들기</div>
    <div class="inline">
      <input id="nl-text" type="text" placeholder="예: 이름에 '계약서'가 들어간 PDF는 문서/계약서로 옮겨줘" ${ai ? '' : 'disabled'} />
      <button class="ghost" id="nl-make" ${ai ? '' : 'disabled'}>Claude로 만들기</button>
    </div>
    <div class="hint">${ai ? 'Claude가 규칙을 채워 주면 확인하고 저장하면 돼요.' : 'Claude Code CLI가 있으면 말로 규칙을 만들 수 있어요. (자동 정리 · Claude 탭)'}</div>
  </div>
  <div class="panel-head">
    <p class="hint">위에 있는 규칙부터 적용돼요. 파일 하나에는 처음 맞는 규칙 하나만 적용돼요.</p>
    <button id="add-rule" class="primary small">+ 직접 만들기</button>
  </div>`;
  const d = state.draft;
  const cards = t.rules.map((r, i) => (d && d.id === r.id ? editorHtml() : ruleCard(r, i, t.rules.length))).join('');
  $('#rules').innerHTML =
    (d && !d.id ? editorHtml() : '') +
    (t.rules.length ? `<div class="rule-list">${cards}</div>` : d ? '' : '<div class="block"><div class="empty">아직 규칙이 없어요. 말로 만들거나 직접 만들어 보세요.</div></div>');
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
    toast('1단계에서 파일 종류나 이름 단어, 기간 중 하나는 골라 주세요.');
    return;
  }
  if (rule.action === 'move' && !rule.dest) {
    toast('2단계에서 옮길 폴더를 골라 주세요.');
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
  toast('규칙을 저장했어요. 맨 위에 넣어서 먼저 적용돼요.');
}

async function makeRuleFromText() {
  const input = $('#nl-text');
  const text = input.value.trim();
  if (!text) return input.focus();
  const btn = $('#nl-make');
  btn.disabled = true;
  btn.innerHTML = '<span class="spinner"></span>만드는 중…';
  try {
    const rule = await tidy.ruleFromText(state.targetId, text);
    state.draft = { ...newDraft(rule), id: null };
    renderRules();
    $('#editor')?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    toast('Claude가 규칙을 채웠어요. 확인하고 저장하세요.');
  } catch (err) {
    toast(`규칙 만들기 실패: ${err.message}`, 6000);
  } finally {
    if ($('#nl-make')) {
      $('#nl-make').disabled = false;
      $('#nl-make').textContent = 'Claude로 만들기';
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
  const who = { 'claude-mcp': '<span class="tag ai">Claude</span>', auto: '<span class="tag auto">자동</span>' };
  $('#history').innerHTML = runs.length
    ? `<div class="block">${runs
        .map((r) => {
          const moved = r.ops.filter((o) => o.type === 'move').length;
          const trashed = r.ops.length - moved;
          const when = new Date(r.at).toLocaleString('ko-KR', { dateStyle: 'medium', timeStyle: 'short' });
          return `<details class="run">
          <summary><span>${who[r.source] || '<span class="tag">앱</span>'} ${esc(when)} · 옮김 ${moved}${trashed ? ` · 휴지통 ${trashed}` : ''}</span>${
            r.undone ? '<span class="tag warn">되돌림</span>' : ''
          }</summary>
          <div class="run-ops">${r.ops
            .map((o) => `<div title="${esc(o.fromLabel)}">${o.type === 'move' ? `${esc(o.fromLabel)} → ${esc(o.toLabel)}` : `휴지통 ← ${esc(o.fromLabel)}`}</div>`)
            .join('')}</div>
        </details>`;
        })
        .join('')}</div>`
    : '<div class="block"><div class="empty">아직 정리한 기록이 없어요.</div></div>';
}

async function doUndo() {
  try {
    const res = await tidy.undo();
    if (!res.ok) return toast(res.message);
    const extra = res.trashed.length ? ` 휴지통으로 보낸 ${res.trashed.length}개는 휴지통에서 직접 꺼내 주세요.` : '';
    const fail = res.failed.length ? ` ${res.failed.length}개는 되돌리지 못했어요(${res.failed[0].reason}).` : '';
    toast(`${res.restored}개 파일을 원래 자리로 돌려놨어요.${extra}${fail}`, 6000);
  } catch (err) {
    toast(`되돌리기 실패: ${err.message}`, 6000);
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
  $('#claude-pill-text').textContent = linked ? 'Claude와 연결됨' : cliReady() ? 'Claude CLI 사용 가능' : 'Claude 연결 안 됨';
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
    [5, '5초'],
    [10, '10초'],
    [30, '30초'],
    [60, '1분'],
    [300, '5분'],
  ];
  const autoCard = `<div class="card">
    <h3>자동 정리</h3>
    <p>켜 둔 폴더에 새 파일이 들어오면, 다 받아질 때까지 기다렸다가 규칙대로 정리해요. 창을 닫아도 ${
      state.platform === 'darwin' ? '메뉴 막대' : '작업 표시줄 알림 영역'
    }에서 계속 지켜봐요. AI 제안은 쓰지 않고 내가 만든 규칙만 써요.</p>
    <div class="settings-list">
      ${cfg.targets
        .map(
          (t) => `<div class="setting-row"><div><b>${esc(targetName(t))}</b><div class="muted">${esc(t.path)} · 켜진 규칙 ${t.rules.filter((r) => r.enabled).length}개</div></div>${switchHtml(
            `data-auto="${esc(t.id)}"`,
            t.auto,
          )}</div>`,
        )
        .join('')}
      <div class="setting-row"><div><b>기다리는 시간</b><div class="muted">파일이 이만큼 그대로면 다 받아진 걸로 봐요</div></div>
        <select id="auto-delay">${delays.map(([v, l]) => `<option value="${v}" ${st.autoDelaySec === v ? 'selected' : ''}>${l}</option>`).join('')}</select></div>
      <div class="setting-row"><div><b>정리할 때 등짝 스매시 효과</b><div class="muted">정리하기를 누르면 손바닥이 "찰싹!!" 내리쳐요. 자동 정리 때는 안 나와요</div></div>${switchHtml('id="set-smash"', st.smash)}</div>
      <div class="setting-row"><div><b>정리한 뒤 폴더 열기</b><div class="muted">정리하기를 누르면 파일이 들어간 폴더를 Finder로 열어 줘요</div></div>${switchHtml('id="set-open"', st.openAfter)}</div>
      <div class="setting-row"><div><b>정리하면 알림 보내기</b></div>${switchHtml('id="set-notify"', st.notify)}</div>
      <div class="setting-row"><div><b>컴퓨터 켤 때 같이 실행</b><div class="muted">창 없이 조용히 시작해요 (설치한 앱에서만 동작)</div></div>${switchHtml('id="set-login"', st.openAtLogin)}</div>
    </div>
  </div>`;

  const s = state.integration;
  if (!s) {
    $('#settings').innerHTML = `${autoCard}<div class="card"><span class="spinner"></span>Claude 연결 확인 중…</div>`;
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
      <h3>Claude 연결 <span class="muted">API 키 없이</span></h3>
      <p>내 PC에 로그인된 Claude를 그대로 빌려 써요. 사용량은 내 Claude 구독에 포함돼요.</p>
      ${
        cli.installed
          ? dotLine(!cli.error, `Claude Code CLI ${esc(cli.version || '')} <span class="muted">${esc(cli.path)}</span>`, `CLI 실행 오류: ${esc(cli.error)}`)
          : dotLine(false, '', 'Claude Code CLI가 없어요')
      }
      <div class="card-actions">
        ${cli.installed ? '' : '<button class="primary small" data-url="https://claude.com/claude-code">설치 방법 보기</button>'}
        <label class="muted">자리 제안·말로 규칙 만들기에 쓸 모델
          <select id="ai-model">${['haiku', 'sonnet', 'opus'].map((m) => `<option value="${m}" ${m === model ? 'selected' : ''}>${m}</option>`).join('')}</select>
        </label>
      </div>
    </div>

    <div class="card">
      <h3>Claude Code에 MCP 서버로 등록</h3>
      <p>등록하면 Claude Code에서 “다운로드 정리해줘”라고 말할 때 이 앱의 규칙·기록을 그대로 써요. 찾은 Claude 설정 위치 모두에 등록해요.</p>
      <div class="loc-list">${(s.locations || [])
        .map((l) => `<div class="loc-row"><span class="dot ${l.registered ? 'on' : 'off'}"></span><span>${esc(l.label)}</span><span class="muted">${l.registered ? '등록됨' : '등록 안 됨'}</span></div>`)
        .join('')}</div>
      <div class="card-actions">
        <button class="primary small" id="reg-code" ${cliReady() ? '' : 'disabled'}>${s.claudeCode ? '다시 등록' : '모두 등록하기'}</button>
        <span class="muted">등록 후 새로 연 Claude Code 세션부터 보여요.</span>
      </div>
      <details class="cmd-details"><summary>직접 등록하는 명령 보기</summary><pre class="cmd">${esc(cmd)}</pre></details>
    </div>

    <div class="card">
      <h3>Claude Desktop에 MCP 서버로 등록</h3>
      <p>Claude Desktop 설정 파일(${esc(s.desktopConfig)})에 추가해요. 기존 파일은 .bak으로 백업하고, 추가한 뒤 Claude Desktop을 다시 시작해야 해요.</p>
      ${dotLine(s.claudeDesktop, '등록됨', s.desktopInstalled ? '등록 안 됨' : 'Claude Desktop이 설치돼 있지 않아요')}
      <div class="card-actions"><button class="primary small" id="reg-desktop" ${s.desktopInstalled ? '' : 'disabled'}>${s.claudeDesktop ? '다시 등록' : '등록하기'}</button></div>
    </div>
    <p class="hint">설정 파일: ${esc(state.configPath)}</p>`;
}

async function register(kind) {
  const btn = kind === 'code' ? $('#reg-code') : $('#reg-desktop');
  btn.disabled = true;
  btn.innerHTML = '<span class="spinner"></span>등록 중…';
  try {
    if (kind === 'code') {
      const results = await tidy.registerClaudeCode();
      const failed = results.filter((r) => !r.ok);
      toast(
        failed.length
          ? `${results.length - failed.length}곳 등록, ${failed.length}곳 실패: ${failed[0].location} — ${failed[0].error || '확인 실패'}`
          : `${results.length}곳에 등록했어요. 새 Claude Code 세션부터 쓸 수 있어요.`,
        6000,
      );
    } else {
      const file = await tidy.registerClaudeDesktop();
      toast(`${file}에 추가했어요. Claude Desktop을 다시 시작해 주세요.`, 5000);
    }
  } catch (err) {
    toast(`등록 실패: ${err.message}`, 6000);
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
    return toast(el.checked ? `${targetName(t)} 자동 정리를 켰어요. 짝! 하고 지켜볼게요.` : `${targetName(t)} 자동 정리를 껐어요.`);
  }
  if (el.id === 'auto-delay') {
    state.config.settings.autoDelaySec = Number(el.value);
    return saveConfig();
  }
  if (['set-notify', 'set-login', 'set-smash', 'set-open'].includes(el.id)) {
    state.config.settings[{ 'set-notify': 'notify', 'set-login': 'openAtLogin', 'set-smash': 'smash', 'set-open': 'openAfter' }[el.id]] = el.checked;
    return saveConfig();
  }
  if (el.id === 'ai-model') {
    state.config.ai = { ...(state.config.ai || {}), model: el.value };
    await saveConfig();
    toast(`Claude 작업에 ${el.value} 모델을 써요.`);
  }
});

tidy.onAutoRan(async ({ count }) => {
  if (count) toast(`짝! 자동으로 ${count}개 치웠어요.`);
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

(async function init() {
  const { config, configPath, home, sep, categories, platform } = await tidy.getConfig();
  Object.assign(state, { config, configPath, home, sep, categories, platform });
  loadFolders();
  refreshIntegration().then(() => {
    if (state.scan) renderPreview();
    if ($('#tab-settings').classList.contains('active')) renderSettings();
    if ($('#tab-rules').classList.contains('active') && !state.draft) renderRules();
  });
  await selectTarget(config.targets[0]?.id || null);
})();
