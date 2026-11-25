'use strict';
/* global tidy, SmashLogic */
// 기록 탭: 최근 7일 합계, "옮긴 파일"(파일 한 줄씩·찾기·Finder에서 보기, #10)과 "정리별"(실행 단위·골라 되돌리기) 두 보기.
// app.js(탭 전환)와 events.js(되돌리기·보기 전환·찾기·자동 정리 알림)가 window.SmashHistory 로 부른다.
(function (root) {
  const { $, L, esc, reducedMotion, toast, ico } = root.SmashUI;
  const { state, revealLabel, trashLabel } = root.SmashState;
  const { doScan } = root.SmashPreview;

  // 보기와 찾는 말은 탭을 오가도 남는다. 처음은 "옮긴 파일"
  let view = 'files';
  let query = '';
  let runs = [];
  const FILES_SHOWN = 200;

  const who = () => ({ 'claude-mcp': '<span class="tag ai">Claude</span>', auto: `<span class="tag auto">${L('자동', 'Auto')}</span>` });
  const whoTag = (source) => who()[source] || `<span class="tag">${L('앱', 'App')}</span>`;

  function renderTools() {
    const seg = `<div class="seg" role="tablist" aria-label="${L('기록 보기', 'History view')}">
      <button class="${view === 'files' ? 'on' : ''}" data-history-view="files" role="tab" aria-selected="${view === 'files'}">${L('옮긴 파일', 'Moved files')}</button>
      <button class="${view === 'runs' ? 'on' : ''}" data-history-view="runs" role="tab" aria-selected="${view === 'runs'}">${L('정리별', 'By run')}</button>
    </div>`;
    const search =
      view === 'files'
        ? `<input id="history-q" type="search" class="history-q" value="${esc(query)}" placeholder="${L('파일 이름이나 폴더로 찾기', 'Find by file name or folder')}" aria-label="${L('옮긴 파일 찾기', 'Find moved files')}" />`
        : '';
    // 찾는 중에 자동 정리로 기록을 다시 그려도 입력칸(글자·커서)은 그대로 둔다
    const box = $('#history-tools');
    if (runs.length && box.dataset.view === view && box.childElementCount) return;
    box.dataset.view = runs.length ? view : '';
    box.innerHTML = runs.length ? seg + search : '';
  }

  function fileRow(f) {
    const when = new Date(f.at).toLocaleString(L('ko-KR', 'en-US'), { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
    const trash = f.type !== 'move';
    const to = trash ? trashLabel() : f.toDirLabel || f.toLabel;
    // 되돌린 파일은 원래 자리에, 옮긴 파일은 옮긴 자리에 있다
    const here = f.undone ? f.from : trash ? '' : f.to;
    const show = here ? `<button class="ghost small" data-show-path="${esc(here)}">${revealLabel()}</button>` : '<span></span>';
    return `<li class="file-row ${f.undone ? 'op-undone' : ''}">
      <span class="ficon">${ico(trash ? 'trash' : 'file')}</span>
      <span class="file-main">
        <span class="fname" title="${esc(f.name)}">${esc(f.name)}</span>
        <span class="file-path" title="${esc(f.fromLabel)} → ${esc(trash ? to : f.toLabel)}">${esc(f.fromDirLabel)} <b>→ ${esc(to)}</b>${
          f.undone ? ` <span class="tag warn">${L('돌려놓음', 'Put back')}</span>` : ''
        }</span>
      </span>
      <span class="file-when">${whoTag(f.source)} ${esc(when)}</span>
      ${show}
    </li>`;
  }

  function filesHtml() {
    const list = SmashLogic.movedFiles(runs, query);
    if (!list.length) {
      return `<div class="block"><div class="empty">${
        query ? L(`"${esc(query)}"에 맞는 파일이 없어요.`, `No files match "${esc(query)}".`) : L('아직 옮긴 파일이 없어요.', 'No files moved yet.')
      }</div></div>`;
    }
    const rest = list.length - FILES_SHOWN;
    return `<div class="block"><ul class="rows">${list.slice(0, FILES_SHOWN).map(fileRow).join('')}</ul>${
      rest > 0 ? `<div class="note">${L(`외 ${rest}개가 더 있어요. 찾기로 좁혀 보세요.`, `${rest} more. Narrow it down with search.`)}</div>` : ''
    }</div>`;
  }

  function renderList() {
    $('#history').innerHTML = view === 'files' && runs.length ? filesHtml() : runsHtml();
  }

  function setView(v) {
    view = v === 'runs' ? 'runs' : 'files';
    renderTools();
    renderList();
  }

  function search(q) {
    query = q;
    renderList(); // 입력칸은 다시 그리지 않는다(글자를 치는 중이라)
  }

  async function renderHistory() {
    const res = await tidy.history();
    runs = res.runs;
    const week = res.week;
    $('#history-hint').textContent = state.canRestoreTrash
      ? L('정리마다 골라서 되돌릴 수 있어요. 휴지통으로 보낸 파일도 휴지통에 남아 있으면 같이 꺼내 와요.', 'Undo any run. Files sent to the Trash come back too, as long as they are still in the Trash.')
      : L('정리마다 골라서 되돌릴 수 있어요. 휴지통으로 보낸 파일은 휴지통에서 직접 꺼내야 해요.', 'Undo any run. Files sent to the Recycle Bin have to be taken out yourself.');
    $('#undo').disabled = !runs.some((r) => SmashLogic.runStatus(r).canUndo);
    $('#week').innerHTML =
      week && week.moved + week.trashed > 0
        ? `<div class="week-line">${L(`최근 7일 동안 ${week.moved + week.trashed}개 치웠어요`, `${week.moved + week.trashed} files tidied in the last 7 days`)}${
            week.trashed ? ` · ${L(`휴지통 ${SmashLogic.fmtBytes(week.trashBytes)}`, `${SmashLogic.fmtBytes(week.trashBytes)} to Trash`)}` : ''
          }</div>`
        : '';
    renderTools();
    renderList();
  }

  // "정리별" 보기: 실행 단위로 접어 두고, 실행마다 되돌리기
  function runsHtml() {
    const stateTag = {
      undone: `<span class="tag warn">${L('되돌림', 'Undone')}</span>`,
      interrupted: `<span class="tag warn" title="${L('정리하다가 앱이 끝났어요. 옮긴 만큼은 되돌릴 수 있어요.', 'The app quit partway. Whatever was moved can still be undone.')}">${L('중단됨', 'Interrupted')}</span>`,
      running: `<span class="tag">${L('정리 중', 'Running')}</span>`,
      failed: `<span class="tag warn">${L('옮긴 것 없음', 'Nothing moved')}</span>`,
    };
    return runs.length
      ? `<div class="block">${runs
          .map((r) => {
            const st = SmashLogic.runStatus(r);
            const moved = r.ops.filter((o) => o.type === 'move').length;
            const trashed = r.ops.length - moved;
            const failed = r.failed || [];
            const when = new Date(r.at).toLocaleString(L('ko-KR', 'en-US'), { dateStyle: 'medium', timeStyle: 'short' });
            const tag = st.key === 'partial' ? `<span class="tag warn">${L(`일부 되돌림 · 남은 ${st.left}개`, `Partly undone · ${st.left} left`)}</span>` : stateTag[st.key] || '';
            const button = st.canUndo
              ? `<button class="ghost small" data-undo-run="${esc(r.id)}">${st.key === 'partial' ? L('남은 것 다시 되돌리기', 'Retry the rest') : L('되돌리기', 'Undo')}</button>`
              : '';
            return `<details class="run">
          <summary><span>${whoTag(r.source)} ${esc(when)} · ${L('옮김', 'moved')} ${moved}${trashed ? ` · ${L('휴지통', 'Trash')} ${trashed}` : ''}${
              failed.length ? ` · ${L('실패', 'failed')} ${failed.length}` : ''
            }</span><span class="run-side">${tag}${button}</span></summary>
          <div class="run-ops">${r.ops
              .map(
                (o) =>
                  `<div class="${o.undone ? 'op-undone' : ''}" title="${esc(o.fromLabel)}">${o.type === 'move' ? `${esc(o.fromLabel)} → ${esc(o.toLabel)}` : `${L('휴지통', 'Trash')} ← ${esc(o.fromLabel)}`}${
                    o.undone ? ` <span class="muted">${L('(돌려놓음)', '(put back)')}</span>` : ''
                  }</div>`,
              )
              .join('')}${failed.map((f) => `<div class="op-failed" title="${esc(f.file)}">${L('못 옮김', 'Not moved')}: ${esc(f.file)} <span class="muted">${esc(f.reason)}</span></div>`).join('')}</div>
        </details>`;
          })
          .join('')}</div>`
      : `<div class="block"><div class="empty">${L('아직 정리한 기록이 없어요.', 'Nothing tidied yet.')}</div></div>`;
  }

  async function doUndo(runId) {
    try {
      const res = await tidy.undo(runId);
      if (!res.ok) return toast(res.message);
      // 방금 정리 결과 띠가 되돌린 실행을 가리키면 지운다(옮긴 자리에 더는 없다)
      if (state.lastResult && (!runId || runId === state.lastResult.runId)) state.lastResult = null;
      // 스매시의 반대 동작: 손바닥으로 가볍게 쓰다듬는다
      if (!reducedMotion()) {
        const icon = $('.brand img');
        icon.classList.remove('pat');
        void icon.offsetWidth;
        icon.classList.add('pat');
      }
      const parts = [L(`미안, 없던 일로. ${res.restored}개를 원래 자리로 돌려놨어요.`, `Sorry, never happened. Put ${res.restored} files back where they were.`)];
      if (res.restoredFromTrash) parts.push(L(`휴지통에서 ${res.restoredFromTrash}개를 꺼냈어요.`, `${res.restoredFromTrash} came back out of the Trash.`));
      if (res.failed.length) {
        parts.push(
          L(
            `${res.failed.length}개는 남겼어요(${res.failed[0].reason}). 기록에서 다시 시도할 수 있어요.`,
            `${res.failed.length} were left (${res.failed[0].reason}). You can retry from the history.`,
          ),
        );
      }
      if (res.trashed.length) parts.push(L(`휴지통으로 보낸 ${res.trashed.length}개는 휴지통에서 직접 꺼내 주세요.`, `Take the ${res.trashed.length} sent to the Trash back out yourself.`));
      toast(parts.join(' '), 7000);
      if (res.trashed.length) tidy.openTrash().catch(() => {});
    } catch (err) {
      toast(L(`되돌리기 실패: ${err.message}`, `Undo failed: ${err.message}`), 6000);
    }
    await renderHistory();
    await doScan();
  }

  root.SmashHistory = { renderHistory, doUndo, setView, search };
})(window);
