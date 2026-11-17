'use strict';
/* global tidy */
// 규칙 탭: 폴더 카드(하위 폴더·연결), 말로 규칙 만들기와 Claude 제안 고르기, 규칙 카드·문장,
// 규칙 편집기(초안·확장자/단어/받은 곳 태그·파일 종류 칩·기간·목적지)와 "목록에서 빼기" 버튼 글자를 맡는다.
// events.js(누름·입력)와 app.js(폴더 고르기·탭 전환)가 window.SmashRules 로 부른다.
(function (root) {
  const { $, L, esc, toast, switchHtml } = root.SmashUI;
  const { state, saveConfig, expand, contract, currentTarget, targetName, cliReady, linkBase, destText, destInside } = root.SmashState;
  const { doScan } = root.SmashPreview;

  function syncRemoveButton() {
    const btn = $('#remove-target');
    if (!btn) return;
    btn.hidden = !state.targetId;
    btn.textContent =
      state.removeArmed && state.removeArmed === state.targetId
        ? L('한 번 더 누르면 목록에서 뺍니다 (파일은 그대로)', 'Click again to remove it from the list (files stay put)')
        : L('이 폴더를 목록에서 빼기', 'Remove this folder from the list');
  }

  function chipList(values, max = 5) {
    const shown = values.slice(0, max).map((v) => `<span class="chip">${esc(v)}</span>`).join('');
    return values.length > max ? `${shown}<span class="chip more">+${values.length - max}</span>` : shown;
  }

  const groupLabel = (g) =>
    ({ auto: L('용도·이름·시간별로 나눠서', 'split by purpose, name & time'), month: L('연·월별로 나눠서', 'split by month') })[g] || '';

  function ruleSentence(r, owner) {
    const parts = [];
    if (r.match.olderThanDays) parts.push(`<span class="chip soft">${L(`받은 지 ${r.match.olderThanDays}일 지난`, `received over ${r.match.olderThanDays} days ago`)}</span>`);
    const froms = r.match.fromIncludes || [];
    if (froms.length) parts.push(L(`${chipList(froms, 3)} 에서 받은`, `from ${chipList(froms, 3)}`));
    if (r.match.nameIncludes.length) parts.push(L(`이름에 ${chipList(r.match.nameIncludes.map((w) => `"${w}"`), 3)} 가 들어간`, `with ${chipList(r.match.nameIncludes.map((w) => `"${w}"`), 3)} in the name`));
    if (r.match.extensions.length) parts.push(chipList(r.match.extensions));
    // 영어는 "확장자 files 기간 이름" 순서가 자연스럽다.
    const ageEn = r.match.olderThanDays ? parts[0] : '';
    const fromEn = froms.length ? parts[r.match.olderThanDays ? 1 : 0] : '';
    const nameEn = r.match.nameIncludes.length ? parts[(r.match.olderThanDays ? 1 : 0) + (froms.length ? 1 : 0)] : '';
    const extEn = r.match.extensions.length ? parts[parts.length - 1] : '';
    const what = parts.length
      ? L(`${parts.join(' ')} 파일은`, [extEn, 'files', ageEn, fromEn, nameEn].filter(Boolean).join(' '))
      : `<span class="warn-text">${L('조건이 없어 적용되지 않아요', 'No conditions, so it never applies')}</span>`;
    const how =
      r.action === 'trash'
        ? L('<b class="to trash">휴지통</b>으로 보내요', 'go to the <b class="to trash">Trash</b>')
        : L(`<b class="to">${esc(destText(r.dest, owner))}</b> 로 옮겨요`, `move to <b class="to">${esc(destText(r.dest, owner))}</b>`);
    const sub = r.action === 'move' && groupLabel(r.group) ? ` <span class="chip soft">${groupLabel(r.group)}</span>` : '';
    const outside =
      r.action === 'move' && !destInside(r.dest, owner)
        ? ` <span class="tag warn" title="${L('정리할 폴더 밖이라 이 규칙은 실행되지 않아요', "Outside your folders, so this rule won't run")}">${L('정리할 폴더 밖', 'Outside your folders')}</span>`
        : '';
    return `${what} → ${how}${sub}${outside}`;
  }

  function ruleCard(r, i, total, owner) {
    if (owner) {
      // 기준 폴더 규칙: 여기서는 보기만 한다
      return `<div class="rule-card readonly ${r.enabled ? '' : 'disabled'}">
      <div class="rule-body">
        <div class="rule-name">${esc(r.name)}${r.enabled ? '' : ` <span class="muted">${L('(꺼짐)', '(off)')}</span>`}</div>
        <div class="rule-sentence">${ruleSentence(r, owner)}</div>
      </div>
    </div>`;
    }
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
    const r = from || { name: '', match: { extensions: [], nameIncludes: [], olderThanDays: 0, fromIncludes: [] }, action: 'move', dest: '', group: 'auto' };
    return {
      id: from?.id || null,
      name: r.name,
      exts: [...r.match.extensions],
      words: [...r.match.nameIncludes],
      froms: [...(r.match.fromIncludes || [])],
      days: r.match.olderThanDays || 0,
      action: r.action,
      dest: r.dest || '',
      group: r.group ?? 'auto',
      matches: null,
    };
  }

  const draftRule = (d) => ({
    name: d.name.trim() || L('새 규칙', 'New rule'),
    enabled: true,
    match: { extensions: d.exts, nameIncludes: d.words, olderThanDays: d.days, fromIncludes: d.froms },
    action: d.action,
    dest: d.action === 'trash' ? '' : d.dest.trim(),
    group: d.action === 'trash' ? '' : d.group,
  });

  const groupOptions = () => [
    ['', L('안 나눔', 'None')],
    ['auto', L('자동 (용도·이름·시간)', 'Auto (purpose, name, time)')],
    ['month', L('연·월', 'By month')],
  ];

  const dayOptions = () => [
    [0, L('바로', 'Right away')],
    [1, L('받은 지 하루', 'A day after it arrives')],
    [7, L('받은 지 일주일', 'A week after')],
    [30, L('받은 지 한 달', 'A month after')],
    [90, L('받은 지 석 달', '3 months after')],
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
    if (!destInside(d.dest)) {
      // 실행 단계는 정리할 폴더 밖으로 옮기지 않는다. 저장하기 전에 알려 준다.
      return `→ <b>${esc(label)}</b> <span class="warn-text">${L('정리할 폴더에 없는 곳이에요. 이대로는 옮겨지지 않아요.', "That isn't one of your folders, so nothing would move there.")}</span>
      <button type="button" class="ghost small" id="dest-add-target">${L('정리할 폴더로 추가', 'Add as a folder')}</button>`;
    }
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
      ${
          state.platform === 'linux'
            ? ''
            : `<div class="field">
        <div class="field-label">${L('받은 곳 주소에 이 단어가 있으면', 'If it was downloaded from')} <span class="muted">${L('(선택)', '(optional)')}</span></div>
        <div class="tag-box">${tags(d.froms, 'from')}<input id="d-from" type="text" placeholder="${L('예: github.com (엔터)', 'e.g. github.com (Enter)')}" /></div>
        <div class="hint">${L('브라우저가 파일에 남긴 내려받은 주소를 봐요. 기록이 없는 파일은 맞지 않아요. 주소는 Claude에 보내지 않아요.', 'Uses the download address the browser saved with the file. Files without one never match. The address is never sent to Claude.')}</div>
      </div>`
        }
      <div class="field">
        <div class="field-label">${L('언제?', 'When?')}</div>
        <div class="seg">${dayOptions().map(([v, l]) => `<button type="button" data-days="${v}" class="${d.days === v ? 'on' : ''}">${l}</button>`).join('')}</div>
        <div class="hint">${L('이 폴더에 들어온 날부터 세요. 그 뒤에 고쳤으면 고친 날부터 세요. 파일에 적힌 옛 날짜는 보지 않아요.', 'Counted from the day the file landed in this folder, or from the last edit if later. The date stored inside the file is ignored.')}</div>
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
            </div>
            <div class="field">
              <div class="field-label">${L('그 폴더 안에서 한 번 더 나누기', 'Split again inside that folder')}</div>
              <div class="seg">${groupOptions().map(([v, l]) => `<button type="button" data-group="${v}" class="${d.group === v ? 'on' : ''}">${l}</button>`).join('')}</div>
              <div class="hint">${L(
                  '자동: 영수증·계약서·스크린샷처럼 용도가 보이는 이름, 이름이 비슷한 파일, 같은 날 찍은 사진끼리 하위 폴더를 만들어요. 이미 만들어 둔 하위 폴더 이름과 맞으면 그 폴더로 넣어요.',
                  'Auto: makes subfolders for names that show a purpose (receipts, contracts, screenshots), files with similar names, and photos taken the same day. Files go into an existing subfolder when its name matches.',
                )}</div>
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
      const hasCondition = rule.match.extensions.length || rule.match.nameIncludes.length || rule.match.fromIncludes.length || rule.match.olderThanDays;
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

  function targetCardHtml(t) {
    const base = linkBase(t);
    const followers = state.config.targets.filter((x) => x.link === t.id);
    const bases = state.config.targets.filter((x) => x.id !== t.id && !x.link);
    const linkControl = followers.length
      ? `<div class="muted">${L(
          `${followers.map((f) => esc(targetName(f))).join(', ')}이(가) 이 폴더를 기준으로 써요. 기준 폴더는 다른 폴더에 다시 연결할 수 없어요.`,
          `${followers.map((f) => esc(targetName(f))).join(', ')} use this folder as their base, so it can't be linked to another one.`,
        )}</div>`
      : `<select id="t-link">
        <option value="">${L('이 폴더 규칙만 쓰기', 'Only this folder’s rules')}</option>
        ${bases.map((b) => `<option value="${esc(b.id)}" ${base?.id === b.id ? 'selected' : ''}>${L(`${esc(targetName(b))} 규칙도 같이 쓰기`, `Also use ${esc(targetName(b))}’s rules`)}</option>`).join('')}
      </select>`;
    return `<div class="card target-card">
    <div class="setting-row">
      <div><b>${L('하위 폴더까지 정리', 'Include subfolders')}</b><div class="muted">${L(
          '켜면 이 폴더 안에 만들어 둔 폴더 속 파일도 규칙대로 다시 정리해요. 앱·사진 보관함 같은 꾸러미와 개발 프로젝트 폴더(.git, package.json 등)는 건드리지 않아요.',
          'When on, files inside folders you made here are re-sorted by the rules too. App/library bundles and code projects (.git, package.json, …) are left alone.',
        )}</div></div>
      ${switchHtml('id="t-recursive"', t.recursive)}
    </div>
    <div class="setting-row">
      <div><b>${L('다른 폴더와 연결', 'Link to another folder')}</b><div class="muted">${L(
          '연결하면 이 폴더 규칙 다음에 기준 폴더 규칙을 이어서 써요. 파일이 기준 폴더의 같은 분류 트리로 모여요.',
          'Linked folders apply their own rules first, then the base folder’s rules, so files land in the base folder’s tree.',
        )}</div></div>
      ${linkControl}
    </div>
  </div>`;
  }

  function proposalHtml() {
    const p = state.proposal;
    if (!p) return '';
    const owner = (id) => state.config.targets.find((t) => t.id === id);
    const rows = p.changes
      .map((c, i) => {
        const o = owner(c.ownerId);
        const tag =
          c.op === 'update'
            ? `<span class="tag warn">${L('고침', 'Update')}</span>`
            : `<span class="tag new">${L('새 규칙', 'New')}</span>`;
        const where = `<span class="muted">${esc(c.ownerName)} · ${c.op === 'update' ? L('제자리', 'in place') : c.position === 'bottom' ? L('맨 아래', 'at the bottom') : L('맨 위', 'at the top')}</span>`;
        return `<label class="proposal-row">
        <input type="checkbox" data-proposal="${i}" ${p.picked.has(i) ? 'checked' : ''} />
        <div>
          <div class="rule-name">${tag} ${esc(c.rule.name)} ${where}</div>
          <div class="rule-sentence">${ruleSentence(c.rule, o)}</div>
          ${c.before ? `<div class="rule-sentence before">${L('지금', 'Now')}: ${ruleSentence(c.before, o)}</div>` : ''}
        </div>
      </label>`;
      })
      .join('');
    return `<div class="card proposal" id="proposal">
    <h3>${L('Claude가 만든 규칙 세트', 'Rule set from Claude')} <span class="muted">${p.changes.length}</span></h3>
    ${p.summary ? `<p>${esc(p.summary)}</p>` : ''}
    <div class="proposal-list">${rows}</div>
    <div class="card-actions">
      <button class="primary small" id="proposal-save" ${p.picked.size ? '' : 'disabled'}>${L(`고른 ${p.picked.size}개 저장`, `Save ${p.picked.size} selected`)}</button>
      <button class="ghost small" id="proposal-cancel">${L('버리기', 'Discard')}</button>
    </div>
  </div>`;
  }

  function renderRules() {
    const t = currentTarget();
    syncRemoveButton();
    if (!t) {
      $('#rules-top').innerHTML = '';
      $('#rules').innerHTML = '';
      return;
    }
    const ai = cliReady();
    const base = linkBase(t);
    $('#rules-top').innerHTML = `${targetCardHtml(t)}
  <div class="nl-card">
    <div class="nl-title">${L('말로 규칙 만들기', 'Describe your rules')}</div>
    <div class="inline">
      <input id="nl-text" type="text" placeholder="${L('예: 사진은 게임 스샷·카톡·카메라로 나누고, 문서는 계약서·영수증·과제로 정리해줘', 'e.g. Split photos into game shots, chats and camera; sort documents into contracts, receipts and homework')}" ${ai ? '' : 'disabled'} />
      <button class="ghost" id="nl-make" ${ai ? '' : 'disabled'}>${L('Claude로 만들기', 'Make it with Claude')}</button>
    </div>
    <div class="hint">${
        ai
          ? L(
              '지금 규칙, 폴더 목록, 파일 종류별 개수와 파일 이름 일부(최대 60개)를 Claude에 보내 대분류·소분류 규칙을 한 벌로 만들어요. 파일 내용은 안 보내요. 겹치는 규칙은 새로 만들지 않고 고치는 쪽으로 보여 줘요.',
              'Sends your current rules, folder list, file counts by type and up to 60 file names (never contents) to Claude to make a full set of categories and subcategories. Overlapping rules show up as updates, not duplicates.',
            )
          : L('Claude Code CLI가 있으면 말로 규칙을 만들 수 있어요. (설정 탭)', 'With the Claude Code CLI you can describe rules in words. (Settings tab)')
      }</div>
  </div>
  ${proposalHtml()}
  <div class="panel-head">
    <p class="hint">${L('위에 있는 규칙부터 적용돼요. 파일 하나에는 처음 맞는 규칙 하나만 적용돼요.', 'Rules apply from the top. Each file gets only the first rule it matches.')}</p>
    <button id="add-rule" class="primary small">${L('+ 직접 만들기', '+ New rule')}</button>
  </div>`;
    const d = state.draft;
    const cards = t.rules.map((r, i) => (d && d.id === r.id ? editorHtml() : ruleCard(r, i, t.rules.length))).join('');
    const own =
      (d && !d.id ? editorHtml() : '') +
      (t.rules.length ? `<div class="rule-list">${cards}</div>` : d ? '' : `<div class="block"><div class="empty">${L('아직 규칙이 없어요. 말로 만들거나 직접 만들어 보세요.', 'No rules yet. Describe one or make one yourself.')}</div></div>`);
    const shared = base
      ? `<div class="panel-head linked-head">
        <p class="hint">${L(`이어서 <b>${esc(targetName(base))}</b> 폴더 규칙을 같이 써요. 목적지는 ${esc(targetName(base))} 폴더 기준이에요.`, `Then <b>${esc(targetName(base))}</b>’s rules apply, with destinations inside ${esc(targetName(base))}.`)}</p>
        <button class="ghost small" data-goto-target="${esc(base.id)}">${L(`${esc(targetName(base))}에서 고치기`, `Edit in ${esc(targetName(base))}`)}</button>
      </div>
      ${base.rules.length ? `<div class="rule-list">${base.rules.map((r, i) => ruleCard(r, i, base.rules.length, base)).join('')}</div>` : `<div class="block"><div class="empty">${L('기준 폴더에 규칙이 없어요.', 'The base folder has no rules.')}</div></div>`}`
      : '';
    $('#rules').innerHTML = own + shared;
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
    const list = { ext: d.exts, word: d.words, from: d.froms }[kind];
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
    if ($('#d-from')?.value.trim()) addTag('from', $('#d-from').value);
    const rule = draftRule(d);
    if (!rule.match.extensions.length && !rule.match.nameIncludes.length && !rule.match.fromIncludes.length && !rule.match.olderThanDays) {
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
    toast(
      rule.action === 'move' && !destInside(rule.dest)
        ? L('규칙을 저장했어요. 다만 목적지가 정리할 폴더 밖이라 아직 옮겨지지 않아요.', "Rule saved, but its destination is outside your folders, so it won't move anything yet.")
        : L('규칙을 저장했어요. 맨 위에 넣어서 먼저 적용돼요.', 'Rule saved at the top, so it applies first.'),
      4000,
    );
  }

  async function makeRuleFromText() {
    const input = $('#nl-text');
    const text = input.value.trim();
    if (!text) return input.focus();
    const btn = $('#nl-make');
    btn.disabled = true;
    btn.innerHTML = `<span class="spinner"></span>${L('만드는 중…', 'Making…')}`;
    try {
      const asked = state.targetId;
      const res = await tidy.ruleFromText(asked, text);
      if (state.targetId !== asked) return;
      state.proposal = { ...res, picked: new Set(res.changes.map((_, i) => i)) };
      state.draft = null;
      renderRules();
      $('#nl-text').value = text;
      $('#proposal')?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
      toast(L('Claude가 규칙 세트를 만들었어요. 고를 것만 남기고 저장하세요.', 'Claude made a rule set. Keep the ones you want and save.'));
    } catch (err) {
      toast(L(`규칙 만들기 실패: ${err.message}`, `Couldn't make the rules: ${err.message}`), 6000);
    } finally {
      if ($('#nl-make')) {
        $('#nl-make').disabled = false;
        $('#nl-make').textContent = L('Claude로 만들기', 'Make it with Claude');
      }
    }
  }

  /** 고른 제안을 규칙에 넣는다: 고침은 제자리에서 바꾸고, 새 규칙은 제안 순서대로 맨 위/맨 아래에. */
  async function saveProposal() {
    const p = state.proposal;
    if (!p) return;
    const picked = p.changes.filter((_, i) => p.picked.has(i));
    const tops = new Map();
    let updated = 0;
    let added = 0;
    for (const c of picked) {
      const owner = state.config.targets.find((t) => t.id === c.ownerId);
      if (!owner) continue;
      if (c.op === 'update') {
        const i = owner.rules.findIndex((r) => r.id === c.rule.id);
        if (i >= 0) {
          owner.rules[i] = { ...c.rule, enabled: owner.rules[i].enabled };
          updated += 1;
          continue;
        }
      }
      const rule = { ...c.rule, id: `r_${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`, enabled: true };
      added += 1;
      if (c.position === 'bottom') owner.rules.push(rule);
      else {
        if (!tops.has(owner)) tops.set(owner, []);
        tops.get(owner).push(rule);
      }
    }
    for (const [owner, rules] of tops) owner.rules.unshift(...rules);
    state.proposal = null;
    await saveConfig();
    renderRules();
    await doScan();
    toast(L(`규칙 ${added}개를 넣고 ${updated}개를 고쳤어요. 미리보기에서 확인하세요.`, `Added ${added} rules and updated ${updated}. Check the preview.`), 4000);
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

  function handleDraftClick(el) {
    const d = state.draft;
    const t = currentTarget();
    if (el.dataset.cat) {
      const cat = state.categories.find((c) => c.key === el.dataset.cat);
      // 일부라도 골라져 있으면(칩이 켜져 보이면) 누를 때 빼고, 하나도 없을 때만 더한다
      const any = cat.extensions.some((x) => d.exts.includes(x));
      d.exts = any ? d.exts.filter((x) => !cat.extensions.includes(x)) : [...new Set([...d.exts, ...cat.extensions])];
      if (!any) {
        if (!d.name.trim() || state.categories.some((c) => c.label === d.name)) d.name = cat.label;
        if (cat.key === 'installer') d.action = 'trash';
        else if (!d.dest && cat.folder) d.dest = expand(t.path) === expand('~/Documents') ? cat.folder : `~/Documents/${cat.folder}`;
      }
      renderEditor();
      return true;
    }
    if (el.dataset.untag) {
      const list = { ext: 'exts', word: 'words', from: 'froms' }[el.dataset.untag];
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
    if (el.dataset.group !== undefined && el.closest('#editor')) {
      d.group = el.dataset.group;
      renderEditor();
      return true;
    }
    return false;
  }

  // ---------- 누름·체크·입력 (events.js 가 부른다) ----------

  function editRule(t, id) {
    return openEditor(t.rules.find((r) => r.id === id));
  }

  async function deleteRule(t, id) {
    t.rules = t.rules.filter((r) => r.id !== id);
    await saveConfig();
    renderRules();
    return doScan();
  }

  /** 규칙 순서 바꾸기. dir 은 -1(먼저) 또는 1(나중에) */
  async function moveRule(t, id, dir) {
    const i = t.rules.findIndex((r) => r.id === id);
    const j = i + Number(dir);
    [t.rules[i], t.rules[j]] = [t.rules[j], t.rules[i]];
    await saveConfig();
    renderRules();
    return doScan();
  }

  async function toggleRule(id, on) {
    const r = currentTarget().rules.find((x) => x.id === id);
    r.enabled = on;
    await saveConfig();
    renderRules();
    return doScan();
  }

  function cancelDraft() {
    state.draft = null;
    return renderRules();
  }

  function cancelProposal() {
    state.proposal = null;
    return renderRules();
  }

  /** 제안 한 줄 고르기. 카드 전체를 다시 그리지 않고 저장 버튼만 맞춘다 */
  function pickProposal(el) {
    const i = Number(el.dataset.proposal);
    if (el.checked) state.proposal.picked.add(i);
    else state.proposal.picked.delete(i);
    const btn = $('#proposal-save');
    btn.disabled = !state.proposal.picked.size;
    btn.textContent = L(`고른 ${state.proposal.picked.size}개 저장`, `Save ${state.proposal.picked.size} selected`);
  }

  // 체크 상태는 저장을 기다린 뒤에도 다시 읽는다(그 사이 또 눌렀으면 마지막 상태로 알린다)
  async function setRecursive(el) {
    const t = currentTarget();
    t.recursive = el.checked;
    await saveConfig();
    toast(
      el.checked
        ? L(`${targetName(t)} 안의 하위 폴더 파일도 정리해요. 미리보기에서 꼭 확인하세요.`, `Files in ${targetName(t)}'s subfolders are included. Check the preview first.`)
        : L('이제 하위 폴더는 건드리지 않아요.', 'Subfolders are left alone now.'),
      4000,
    );
    return doScan();
  }

  async function setLink(value) {
    const t = currentTarget();
    t.link = value;
    await saveConfig();
    renderRules();
    return doScan();
  }

  const TAG_INPUTS = { 'd-ext': 'ext', 'd-word': 'word', 'd-from': 'from' };
  const isTagInput = (el) => Object.prototype.hasOwnProperty.call(TAG_INPUTS, el.id); // Object.hasOwn 은 Safari 15.4+ 라 macOS 11 최소 사양에서 못 씀

  /** 태그 입력칸: 엔터·쉼표로 태그를 넣고, 빈 칸에서 지우기를 누르면 마지막 태그를 뺀다 */
  function tagKeydown(e) {
    const el = e.target;
    const kind = TAG_INPUTS[el.id];
    if ((e.key === 'Enter' || e.key === ',') && el.value.trim()) {
      e.preventDefault();
      addTag(kind, el.value);
      return renderEditor(el.id);
    }
    if (e.key === 'Backspace' && !el.value) {
      const list = { ext: state.draft.exts, word: state.draft.words, from: state.draft.froms }[kind];
      if (list.length) {
        list.pop();
        return renderEditor(el.id);
      }
    }
  }

  /** 편집기 이름·목적지 입력. 목적지는 고칠 때마다 아래 미리보기 글자를 맞춘다 */
  function draftInput(e) {
    const d = state.draft;
    if (!d) return;
    if (e.target.id === 'd-name') d.name = e.target.value;
    if (e.target.id === 'd-dest') {
      d.dest = e.target.value;
      $('#dest-preview').innerHTML = destPreviewHtml(d);
    }
  }

  root.SmashRules = {
    syncRemoveButton,
    renderRules,
    renderEditor,
    openEditor,
    handleDraftClick,
    saveDraft,
    makeRuleFromText,
    saveProposal,
    pickDest,
    loadFolders,
    editRule,
    deleteRule,
    moveRule,
    toggleRule,
    cancelDraft,
    cancelProposal,
    pickProposal,
    setRecursive,
    setLink,
    isTagInput,
    tagKeydown,
    draftInput,
  };
})(window);
