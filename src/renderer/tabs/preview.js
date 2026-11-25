'use strict';
/* global tidy, SmashLogic, SmashSound */
// 미리보기 탭: 스캔해서 목적지 기준 폴더 트리로 그리고, 체크·접기, 막힌 파일·방금 결과·처음 안내,
// Claude 자리 묻기·사진 내용 묶기, 정리 실행(짝!)과 타격 순간 효과를 맡는다.
// events.js(누름·체크)와 app.js(폴더 고르기)·다른 탭(저장 뒤 다시 스캔)이 window.SmashPreview 로 부른다.
(function (root) {
  const { $, L, esc, reducedMotion, toast, fmtSize, ico } = root.SmashUI;
  const { state, revealLabel, saveConfig, baseName, dirOf, expand, contract, targetName, cliReady, destText } = root.SmashState;

  async function doScan() {
    if (!state.targetId) return;
    state.aiItems = [];
    state.aiNoSuggestion = [];
    state.unchecked = new Set();
    $('#plan').innerHTML = `<div class="block"><div class="empty"><span class="spinner"></span>${L('살펴보는 중…', 'Looking…')}</div></div>`;
    $('#unmatched').innerHTML = '';
    $('#blocked').innerHTML = '';
    try {
      state.scan = await tidy.scan(state.targetId);
    } catch (err) {
      state.scan = null;
      $('#plan').innerHTML = `<div class="block"><div class="empty">${esc(err.message)}</div></div>`;
      $('#target-sub').textContent = '';
      updateApplyButton();
      return;
    }
    // 파일이 아주 많으면 하위 폴더는 접어서 그린다(펼칠 때 그 안의 행을 만든다)
    state.autoCollapse = state.scan.items.length > 400;
    if (!state.autoCollapse) state.collapsed = new Set([...state.collapsed].filter((k) => !k.startsWith('auto:')));
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

  const protectedCount = (scan) => (scan.blocked || []).filter((it) => it.note === 'protected').length;
  const outsideCount = (scan) => (scan.blocked || []).length - protectedCount(scan);

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
    ].filter(Boolean);
    const where =
      it.note === 'duplicate'
        ? L(`같은 파일이 ${esc(destText(dirOf(it.duplicateOf)))}에 있어요`, `Same file already in ${esc(destText(dirOf(it.duplicateOf)))}`)
        : `${sameDir ? '' : L(`${esc(fromDir)}에서 · `, `from ${esc(fromDir)} · `)}${esc(String(it.reason || '').replace(/^규칙: /, ''))}`;
    // 여러 규칙에 맞으면 위에 있는 규칙이 잡는다. 밀린 규칙을 알려 준다.
    const also = it.alsoMatched || [];
    const why = also.length
      ? L(
          `규칙 '${it.reason}'이(가) 먼저 잡았어요. ${also.map((n) => `'${n}'`).join(', ')}도 맞지만 순서가 뒤예요. 규칙 탭에서 순서를 바꿀 수 있어요.`,
          `Rule '${it.reason}' caught it first. ${also.map((n) => `'${n}'`).join(', ')} also match but come later. Reorder them in the Rules tab.`,
        )
      : '';
    if (also.length) tags.push(`<span class="tag" title="${esc(why)}">${L(`규칙 ${also.length + 1}개에 맞음`, `${also.length + 1} rules match`)}</span>`);
    return `<li class="trow file ${off ? 'off' : ''}" data-depth="${Math.min(depth, 8)}" data-row="${esc(it.id)}">
    <span class="caret-space"></span>
    <input type="checkbox" data-item="${esc(it.id)}" ${off ? '' : 'checked'} aria-label="${L(`${esc(it.name)} 포함`, `Include ${esc(it.name)}`)}" />
    <span class="ficon">${ico('file')}</span>
    <span class="tname" title="${esc(it.srcLabel)}">${esc(it.name)}</span>
    <span class="tmeta">${where} · ${fmtSize(it.size)}</span>
    <span class="tags">${tags.join('')}</span>
  </li>`;
  }

  function folderRow(node, depth, newDirs, { icon = 'folder', isNew } = {}) {
    const ids = collectIds(node);
    state.folderItems.set(node.abs, ids);
    // 파일이 아주 많을 때는 하위 폴더를 접힌 채로 시작한다. 한 번 펼친 폴더는 그대로 둔다.
    if (state.autoCollapse && depth >= 1 && !state.collapsed.has(`auto:${node.abs}`)) {
      state.collapsed.add(`auto:${node.abs}`);
      state.collapsed.add(node.abs);
    }
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
    renderIntro();
    renderResult();
    if (!scan) {
      $('#plan').innerHTML = '';
      $('#unmatched').innerHTML = '';
      $('#blocked').innerHTML = '';
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
      outsideCount(scan) ? L(`옮길 수 없음 ${outsideCount(scan)}개`, `${outsideCount(scan)} can't move`) : '',
      protectedCount(scan) ? L(`보호 ${protectedCount(scan)}개`, `${protectedCount(scan)} protected`) : '',
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
        ${scan.truncated ? `<div class="block-sub warn-text">${L('파일이 많아 20,000개(폴더 깊이 8단계)까지만 봤어요. 정리한 뒤 다시 보기를 눌러 주세요.', 'There are a lot of files, so only the first 20,000 (8 folders deep) were checked. Tidy, then press Refresh.')}</div>` : ''}
      </div>
      <div class="block-actions">
        ${visionButton(items)}
        ${items.length ? `<button class="link" id="toggle-all">${L('전체 선택/해제', 'Select all / none')}</button>` : ''}
      </div>
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
            <div class="block-sub">${L('얘네는 어디 둘지 모르겠어요. ', "Not sure where these go. ")}${L('Claude에게는 파일 이름·크기·날짜와 폴더 목록만 보내요. 파일 내용은 안 보내요.', 'Claude only gets file names, sizes, dates and your folder list, never file contents.')}</div>
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
    renderBlocked();
    updateApplyButton();
  }

  // 규칙에는 맞지만 목적지가 정리할 폴더 밖이라 옮길 수 없는 파일. 계획에 넣지 않고 이유와 고치는 길을 보여 준다.
  function renderBlocked() {
    const all = state.scan?.blocked || [];
    // 정리 대상이 git 저장소 안이면 경고만 한다(#8). 막지는 않는다.
    const repoHtml = state.scan?.repo
      ? `<div class="block"><div class="block-head"><div>
        <div class="block-title">${L('개발 저장소 안의 폴더예요', 'This folder is inside a code repository')}</div>
        <div class="block-sub">${L(`위쪽 폴더 ${esc(state.scan.repo)} 가 git 저장소예요. 여기 파일을 옮기면 프로젝트에 영향이 갈 수 있어요. 설정 파일·코드는 보호하지만, 그 밖의 파일은 규칙대로 옮겨요.`, `${esc(state.scan.repo)} above this folder is a git repository. Moving files here can affect the project. Config and code files stay protected; other files follow your rules.`)}</div>
      </div></div></div>`
      : '';
    // 코드·프로젝트 파일이라 보호한 것은 따로 짧게 보여 준다(#8)
    const kept = all.filter((it) => it.note === 'protected');
    const blocked = all.filter((it) => it.note !== 'protected');
    const keptHtml = kept.length
      ? `<div class="block">
    <div class="block-head">
      <div>
        <div class="block-title">${L('보호해서 그대로 둔 파일', 'Protected and left in place')}<span class="count">${kept.length}</span></div>
        <div class="block-sub">${L('코드·프로젝트 설정 파일이나 개발 프로젝트 안 파일은 규칙에 맞아도 옮기지 않아요. 옮기면 프로젝트가 깨질 수 있어요.', "Code, project config files and files inside dev projects stay put even when a rule matches. Moving them can break the project.")}</div>
      </div>
    </div>
    <ul class="rows">${kept
        .slice(0, 8)
        .map((it) => `<li class="plain-row"><span class="ficon">${ico('file')}</span><span class="fname" title="${esc(it.srcLabel)}">${esc(it.name)}</span><span class="dest">${esc(String(it.reason || '').replace(/^[^:：]*[:：]\s*/, ''))}</span></li>`)
        .join('')}</ul>
    ${kept.length > 8 ? `<div class="note">${L(`외 ${kept.length - 8}개`, `and ${kept.length - 8} more`)}</div>` : ''}
  </div>`
      : '';
    if (!blocked.length) {
      $('#blocked').innerHTML = repoHtml + keptHtml;
      return;
    }
    const byDir = new Map();
    for (const it of blocked) {
      const dir = dirOf(it.dest);
      if (!byDir.has(dir)) byDir.set(dir, []);
      byDir.get(dir).push(it);
    }
    $('#blocked').innerHTML = `<div class="block">
    <div class="block-head">
      <div>
        <div class="block-title">${L('옮길 수 없는 파일', "Can't be moved")}<span class="count">${blocked.length}</span></div>
        <div class="block-sub">${L('규칙의 목적지가 정리할 폴더에 없는 곳이에요. Smash!는 정리할 폴더 밖으로는 옮기지 않아요. 그 폴더를 정리할 폴더로 추가하거나 규칙의 목적지를 바꿔 주세요.', "These rules point to a place that isn't one of your folders. Smash! never moves files outside them. Add that folder or change the rule's destination.")}</div>
      </div>
    </div>
    ${[...byDir.entries()]
        .map(
          ([dir, items]) => `<div class="blocked-group">
        <div class="blocked-head"><span>→ <b>${esc(contract(dir))}</b> <span class="muted">${L(`규칙 '${esc(items[0].reason)}'`, `rule '${esc(items[0].reason)}'`)}</span></span>
          <button class="ghost small" data-add-target-path="${esc(dir)}">${L('정리할 폴더로 추가', 'Add as a folder')}</button></div>
        <ul class="rows">${items
            .slice(0, 8)
            .map((it) => `<li class="plain-row"><span class="ficon">${ico('file')}</span><span class="fname" title="${esc(it.srcLabel)}">${esc(it.name)}</span><span class="dest">${fmtSize(it.size)}</span></li>`)
            .join('')}</ul>
        ${items.length > 8 ? `<div class="note">${L(`외 ${items.length - 8}개`, `and ${items.length - 8} more`)}</div>` : ''}
      </div>`,
        )
        .join('')}
  </div>${keptHtml}`;
    if (repoHtml) $('#blocked').insertAdjacentHTML('afterbegin', repoHtml);
  }

  // 방금 정리한 결과: 옮긴 파일과 옮긴 곳(#10), 옮기지 못한 파일. 닫기 전까지 남아 있다.
  const MOVED_SHOWN = 30;

  function movedRow(m) {
    const where =
      m.type === 'move'
        ? `<span class="dest" title="${esc(m.toDirLabel)}">→ ${esc(m.toDirLabel)}</span><button class="ghost small" data-show-path="${esc(m.to)}">${revealLabel()}</button>`
        : `<span class="dest">→ ${L('휴지통', 'Trash')}</span><span></span>`;
    return `<li class="plain-row moved-row"><span class="ficon">${ico('file')}</span><span class="fname" title="${esc(m.name)}">${esc(m.name)}</span>${where}</li>`;
  }

  function renderResult() {
    const r = state.lastResult;
    const moved = r?.moved || [];
    if (!r || (!r.failed.length && !r.historyError && !moved.length)) {
      $('#result').innerHTML = '';
      return;
    }
    const head = r.failed.length
      ? `<b class="warn-text">${L(`${r.failed.length}개는 그대로 있어요`, `${r.failed.length} files stayed where they were`)}</b> <span class="muted">${L(`${r.ok}개는 치웠어요`, `${r.ok} were tidied`)}</span>`
      : `<b>${L(`${r.ok}개 치웠어요`, `Tidied ${r.ok} files`)}</b> <span class="muted">${esc(r.destLine || '')}</span>`;
    const rest = moved.length - MOVED_SHOWN;
    $('#result').innerHTML = `<details class="block result-band" open>
    <summary><span class="result-head">${head}</span>
      <span class="run-side"><button class="link" data-goto="history">${L('기록에서 보기', 'See in History')}</button><button class="link" id="result-close">${L('닫기', 'Dismiss')}</button></span></summary>
    ${r.historyError ? `<div class="note warn-text">${esc(r.historyError)} ${L('기록을 남길 수 없으면 되돌릴 수 없어서 옮기지 않았어요.', "Without a history entry it couldn't be undone, so nothing more was moved.")}</div>` : ''}
    ${r.failed.length ? `<ul class="rows">${r.failed.map((f) => `<li class="plain-row"><span class="ficon">${ico('file')}</span><span class="fname">${esc(f.name)}</span><span class="dest">${esc(f.reason)}</span></li>`).join('')}</ul>` : ''}
    ${moved.length ? `<ul class="rows">${moved.slice(0, MOVED_SHOWN).map(movedRow).join('')}</ul>` : ''}
    ${rest > 0 ? `<div class="note">${L(`외 ${rest}개는 기록 탭에서 볼 수 있어요.`, `${rest} more in the History tab.`)}</div>` : ''}
  </details>`;
  }

  // 처음 켰을 때 한 번 보여 주는 안내. 소리가 난다는 것도 여기서 알린다.
  function renderIntro() {
    const show = state.config && !state.config.settings.introSeen;
    $('#intro').innerHTML = show
      ? `<div class="block intro">
        <div class="intro-text">
          <b>${L('처음이시죠?', 'First time here?')}</b>
          <span>${L('아래 미리보기에서 확인하고 "짝! 정리하기"를 누르면 그대로 옮겨요. 옮긴 건 기록 탭에서 되돌릴 수 있어요.', 'Check the preview below, then press "Smack! Tidy up". Anything moved can be undone in the History tab.')}</span>
          <span>${L('정리할 때 "찰싹" 소리가 나요. 설정 탭에서 끌 수 있어요.', 'Tidying makes a smack sound. You can turn it off in Settings.')}</span>
        </div>
        <button class="ghost small" id="intro-ok">${L('알겠어요', 'Got it')}</button>
      </div>`
      : '';
  }

  const MEDIA_EXT = /\.(jpe?g|png|gif|heic|heif|webp|tiff?|bmp|mp4|mov|m4v|avi|mkv|webm)$/i;

  const mediaItems = (items) => items.filter((it) => it.action === 'move' && it.groupBase && MEDIA_EXT.test(it.name));

  function visionButton(items) {
    const n = mediaItems(items).length;
    if (!n || !state.config.ai?.vision || !cliReady()) return '';
    return `<button class="ghost small" id="ai-group" title="${L('사진·동영상 썸네일을 Claude에 보여 주고 비슷한 것끼리 하위 폴더로 묶어요', 'Shows photo & video thumbnails to Claude and groups similar ones into subfolders')}">${L(
      `Claude로 사진·영상 ${n}개 내용 보고 묶기`,
      `Group ${n} photos & videos by content`,
    )}</button>`;
  }

  async function aiGroupMedia() {
    const btn = $('#ai-group');
    btn.disabled = true;
    btn.innerHTML = `<span class="spinner"></span>${L('Claude가 사진을 보는 중…', 'Claude is looking at them…')}`;
    // 오래 걸리는 동안 폴더를 바꾸거나 다시 보기를 누르면 그 결과는 버린다(다른 폴더 계획을 덮어쓰지 않게)
    const scan = state.scan;
    try {
      const res = await tidy.groupMedia(scan.items);
      if (state.scan !== scan) return;
      state.scan.items = res.items;
      // 다시 계획하면서 빠진 항목(이미 그 자리에 있음)은 체크 목록에서도 정리한다
      const ids = new Set(allPlanItems().map((i) => i.id));
      state.unchecked = new Set([...state.unchecked].filter((id) => ids.has(id)));
      renderPreview();
      toast(
        res.grouped
          ? L(`Claude가 ${res.grouped}개를 내용별로 묶었어요. 트리에서 확인하세요.`, `Claude grouped ${res.grouped} by content. Check the tree.`)
          : L('Claude가 묶을 만한 걸 찾지 못했어요.', "Claude didn't find anything to group."),
        4000,
      );
    } catch (err) {
      if (state.scan !== scan || !$('#ai-group')) return;
      $('#ai-group').disabled = false;
      $('#ai-group').textContent = L('다시 시도', 'Try again');
      toast(L(`묶기 실패: ${err.message}`, `Grouping failed: ${err.message}`), 6000);
    }
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
    state.lastResult = null;
    SmashSound.warm();
    updateApplyButton();
    const btn = $('#apply');
    btn.classList.add('struck');
    setTimeout(() => btn.classList.remove('struck'), 140);
    btn.innerHTML = `<span class="spinner"></span>${L('정리 중…', 'Tidying…')}`;
    // 손바닥이 내리치는 순간에 맞은 행을 뺀다
    state.struckIds = items.map((it) => it.id);
    try {
      const { results, historyError, moved, destLine } = await tidy.apply(items);
      const ok = results.filter((r) => r.ok).length;
      const failed = SmashLogic.failures(results);
      state.lastResult = { ok, failed, historyError: historyError || null, moved: moved || [], destLine: destLine || '' };
      toast(
        failed.length
          ? L(`${ok}개 정리, ${failed.length}개는 그대로 있어요. 미리보기 위에서 이유를 볼 수 있어요.`, `Tidied ${ok}, ${failed.length} stayed put. See why above the preview.`)
          : L(`짝! ${ok}개 치웠어요. 기록 탭에서 되돌릴 수 있어요.`, `Smack! Tidied ${ok} files. You can undo it in the History tab.`),
        failed.length ? 6000 : 3500,
      );
    } catch (err) {
      toast(L(`실행 실패: ${err.message}`, `Failed: ${err.message}`), 6000);
    } finally {
      state.busy = false;
      state.struckIds = null;
      await doScan();
    }
  }

  // 타격 순간: 소리를 내고, 정리 중인 행을 위에서부터 납작하게 뺀다.
  function onHit({ level }) {
    if (state.config?.settings.sound !== false) SmashSound.play(level);
    const ids = state.struckIds;
    if (!ids || reducedMotion()) return;
    // 화면에 보이는 순서(위에서부터)로 뺀다
    const struck = new Set(ids);
    const rows = [...document.querySelectorAll('[data-row]')].filter((row) => struck.has(row.dataset.row));
    SmashLogic.staggerDelays(rows.length).forEach((delay, i) => {
      rows[i].style.animationDelay = `${delay}ms`;
      rows[i].classList.add('slapped');
    });
  }

  function onProgress({ done, total }) {
    // 금방 끝나는 정리는 숫자를 보여 주지 않는다
    if (!state.busy || total < 30) return;
    $('#apply').innerHTML = `<span class="spinner"></span>${L('정리 중…', 'Tidying…')} ${done.toLocaleString()} / ${total.toLocaleString()}`;
  }

  // ---------- 누름·체크 (events.js 가 부른다) ----------

  /** 폴더 옆 ▾: 접고 펼친다 */
  function toggleCollapse(key) {
    if (state.collapsed.has(key)) state.collapsed.delete(key);
    else state.collapsed.add(key);
    return renderPreview();
  }

  /** 전체 선택/해제: 하나라도 빠져 있으면 모두 넣고, 아니면 모두 뺀다 */
  function toggleAll() {
    if (state.unchecked.size) state.unchecked.clear();
    else allPlanItems().forEach((it) => state.unchecked.add(it.id));
    return renderPreview();
  }

  /** 파일 한 줄의 체크. 트리 전체를 다시 그리지 않고 그 줄과 폴더 체크만 맞춘다 */
  function checkItem(el) {
    if (el.checked) state.unchecked.delete(el.dataset.item);
    else state.unchecked.add(el.dataset.item);
    el.closest('.trow').classList.toggle('off', !el.checked);
    syncFolderChecks();
    return updateApplyButton();
  }

  /** 폴더 체크: 그 아래 파일을 모두 넣거나 뺀다 */
  function checkFolder(el) {
    const ids = state.folderItems.get(el.dataset.folder) || [];
    ids.forEach((id) => (el.checked ? state.unchecked.delete(id) : state.unchecked.add(id)));
    return renderPreview();
  }

  /** 방금 정리 결과 띠 닫기 */
  function closeResult() {
    state.lastResult = null;
    return renderResult();
  }

  /** 처음 안내 "알겠어요" */
  function dismissIntro() {
    state.config.settings.introSeen = true;
    renderIntro();
    return saveConfig();
  }

  root.SmashPreview = {
    doScan,
    renderPreview,
    aiGroupMedia,
    aiSuggest,
    doApply,
    onHit,
    onProgress,
    toggleCollapse,
    toggleAll,
    checkItem,
    checkFolder,
    closeResult,
    dismissIntro,
  };
})(window);
