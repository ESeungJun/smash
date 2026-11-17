'use strict';
/* global tidy, SmashLogic */
// 기록 탭: 정리 기록 목록·최근 7일 합계를 그리고, 골라 되돌리기·마지막 되돌리기를 한다.
// app.js(탭 전환)와 events.js(되돌리기 버튼·자동 정리 알림)가 window.SmashHistory 로 부른다.
(function (root) {
  const { $, L, esc, reducedMotion, toast } = root.SmashUI;
  const { state } = root.SmashState;
  const { doScan } = root.SmashPreview;

  async function renderHistory() {
    const { runs, week } = await tidy.history();
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
    const who = { 'claude-mcp': '<span class="tag ai">Claude</span>', auto: `<span class="tag auto">${L('자동', 'Auto')}</span>` };
    const stateTag = {
      undone: `<span class="tag warn">${L('되돌림', 'Undone')}</span>`,
      interrupted: `<span class="tag warn" title="${L('정리하다가 앱이 끝났어요. 옮긴 만큼은 되돌릴 수 있어요.', 'The app quit partway. Whatever was moved can still be undone.')}">${L('중단됨', 'Interrupted')}</span>`,
      running: `<span class="tag">${L('정리 중', 'Running')}</span>`,
      failed: `<span class="tag warn">${L('옮긴 것 없음', 'Nothing moved')}</span>`,
    };
    $('#history').innerHTML = runs.length
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
          <summary><span>${who[r.source] || `<span class="tag">${L('앱', 'App')}</span>`} ${esc(when)} · ${L('옮김', 'moved')} ${moved}${trashed ? ` · ${L('휴지통', 'Trash')} ${trashed}` : ''}${
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

  root.SmashHistory = { renderHistory, doUndo };
})(window);
