'use strict';
/* global tidy */
// 본 화면의 뼈대: 사이드바의 정리할 폴더 목록(그리기·고르기·더하기·빼기), 탭 전환, index.html 고정 문구 번역, 시작(init).
// 탭 내용은 tabs/*.js, 이벤트 연결은 events.js 가 맡는다. index.html 에서 맨 마지막에 싣는다.
// 시작 순서(원래와 같다): 저장 뒤 사이드바 다시 그리기 연결 → 이벤트 연결(SmashEvents.bind) → init.
(function (root) {
  const { $, L, esc, toast, getLang, setLang } = root.SmashUI;
  const { state, adoptConfig, saveConfig, baseName, expand, contract, currentTarget, targetName } = root.SmashState;
  const { doScan, renderPreview } = root.SmashPreview;
  const { renderRules, syncRemoveButton, loadFolders } = root.SmashRules;
  const { renderHistory } = root.SmashHistory;
  const { renderSettings, refreshIntegration } = root.SmashSettings;

  // ---------- 사이드바: 정리할 폴더 목록 ----------

  function renderTargets() {
    $('#targets').innerHTML = state.config.targets
      .map((t) => {
        const armed = state.removeArmed === t.id;
        const base = t.link && state.config.targets.find((x) => x.id === t.link);
        return `<div class="target-row ${t.id === state.targetId ? 'active' : ''}">
        <button class="target-btn" data-id="${esc(t.id)}" title="${esc(t.path)}">
          <span>${esc(targetName(t))}</span>${base ? `<span class="link-badge" title="${L(`${esc(targetName(base))} 규칙을 같이 써요`, `Shares rules with ${esc(targetName(base))}`)}">↗ ${esc(targetName(base))}</span>` : ''}${
              t.auto ? `<span class="auto-badge" title="${L('자동 정리 켜짐', 'Auto-tidy on')}">${L('자동', 'Auto')}</span>` : ''
            }
        </button>
        <button class="target-x ${armed ? 'armed' : ''}" data-remove-target="${esc(t.id)}" title="${L('목록에서 빼기 (파일은 그대로)', 'Remove from the list (files stay put)')}" aria-label="${L(`${esc(targetName(t))} 목록에서 빼기`, `Remove ${esc(targetName(t))} from the list`)}">${armed ? L('빼기?', 'Remove?') : '×'}</button>
      </div>`;
      })
      .join('');
  }

  async function selectTarget(id) {
    state.targetId = id;
    state.draft = null;
    state.proposal = null;
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
    state.config.targets.push({ id, path: label, auto: false, rules: [], autoTrash: true });
    await saveConfig();
    await selectTarget(id);
    toast(L('폴더를 추가했어요. 규칙 탭에서 정리 규칙을 만들어 보세요.', 'Folder added. Make some rules in the Rules tab.'));
  }

  let removeArmTimer;
  /** 폴더를 목록에서 뺀다. 한 번 누르면 확인 상태, 3초 안에 한 번 더 누르면 뺀다. 파일은 건드리지 않는다. */
  async function removeTarget(id) {
    const t = state.config.targets.find((x) => x.id === id);
    if (!t) return;
    if (state.removeArmed !== id) {
      state.removeArmed = id;
      renderTargets();
      syncRemoveButton();
      clearTimeout(removeArmTimer);
      removeArmTimer = setTimeout(() => {
        state.removeArmed = null;
        renderTargets();
        syncRemoveButton();
      }, 3000);
      return;
    }
    clearTimeout(removeArmTimer);
    state.removeArmed = null;
    state.config.targets = state.config.targets.filter((x) => x.id !== id);
    // 이 폴더를 기준으로 쓰던 폴더는 연결을 끊는다(규칙은 각자 그대로)
    state.config.targets.forEach((x) => {
      if (x.link === id) x.link = '';
    });
    await saveConfig();
    toast(L(`${targetName(t)}을(를) 목록에서 뺐어요. 파일은 그대로예요.`, `Removed ${targetName(t)} from the list. Files stay put.`));
    if (state.targetId !== id) {
      // 지금 보는 폴더가 뺀 폴더를 기준으로 쓰고 있었을 수 있어 계획을 다시 짠다
      syncRemoveButton();
      renderRules();
      return doScan();
    }
    state.targetId = state.config.targets[0]?.id || null;
    if (state.targetId) await selectTarget(state.targetId);
    else {
      state.scan = null;
      renderTargets();
      renderPreview();
      renderRules();
      $('#target-path').textContent = L('폴더를 추가해 주세요', 'Add a folder');
      $('#target-sub').textContent = '';
    }
    syncRemoveButton();
  }

  /** 경로 하나를 정리할 폴더로 더한다(이미 있으면 그대로). 미리보기의 막힌 파일·편집기의 밖 목적지에서 부른다. */
  async function addTargetPath(path) {
    const label = contract(path);
    if (!state.config.targets.some((t) => expand(t.path) === path)) {
      state.config.targets.push({ id: `t_${Date.now().toString(36)}`, path: label, auto: false, rules: [], autoTrash: true });
      await saveConfig();
      toast(L(`${baseName(path)}을(를) 정리할 폴더로 추가했어요. 이제 그 폴더로 옮길 수 있어요.`, `Added ${baseName(path)} as a folder. Files can move there now.`), 4000);
    }
    renderRules();
    await doScan();
  }

  // ---------- 탭 전환·고정 문구 ----------

  function switchTab(name) {
    document.querySelectorAll('.tab').forEach((t) => t.classList.toggle('active', t.dataset.tab === name));
    document.querySelectorAll('.panel').forEach((p) => p.classList.toggle('active', p.id === `tab-${name}`));
    if (name === 'history') renderHistory();
    if (name === 'settings') renderSettings();
    if (name === 'rules') renderRules();
  }

  // index.html에 적힌 고정 문구(한국어)를 화면 언어에 맞춘다.
  function translateStatic() {
    document.documentElement.lang = getLang();
    if (getLang() === 'ko') return;
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
    set('.tab[data-tab="settings"]', 'Settings');
    set('#remove-target', 'Remove this folder from the list');
    set('#reset-rules', 'Reset rules');
    set('#undo', 'Undo last run');
  }

  // ---------- 시작 ----------

  async function init() {
    const { config, configPath, home, sep, categories, platform, lang, rev, canRestoreTrash } = await tidy.getConfig();
    Object.assign(state, { configPath, home, sep, categories, platform, canRestoreTrash: canRestoreTrash !== false });
    adoptConfig(config, rev);
    setLang(lang);
    translateStatic();
    tidy.updateStatus().then((u) => {
      state.update = u;
      if ($('#tab-settings').classList.contains('active')) renderSettings();
    });
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
  }

  // 저장할 때마다 사이드바 폴더 목록을 다시 그린다
  root.SmashState.onSaved(() => renderTargets());
  // 규칙 초기화(#11): 한 번 누르면 무엇이 바뀌는지 알리고, 3초 안에 한 번 더 누르면 실행한다
  let resetArmTimer = null;
  function syncResetButton() {
    const btn = $('#reset-rules');
    const hint = $('#reset-rules-hint');
    btn.textContent = state.resetArmed ? L('한 번 더 누르면 초기화해요', 'Click again to reset') : L('규칙 초기화', 'Reset rules');
    btn.classList.toggle('armed', state.resetArmed);
    hint.hidden = !state.resetArmed;
    hint.textContent = L(
      '정리할 폴더가 다운로드·문서 두 개와 기본 규칙으로 돌아가요. 파일·정리 기록·앱 설정은 그대로예요.',
      'Your folders go back to Downloads and Documents with the default rules. Files, history and settings stay as they are.',
    );
  }

  async function resetRules() {
    if (!state.resetArmed) {
      state.resetArmed = true;
      syncResetButton();
      clearTimeout(resetArmTimer);
      resetArmTimer = setTimeout(() => {
        state.resetArmed = false;
        syncResetButton();
      }, 3000);
      return;
    }
    clearTimeout(resetArmTimer);
    state.resetArmed = false;
    syncResetButton();
    try {
      const { config, rev } = await tidy.resetRules();
      adoptConfig(config, rev);
      state.draft = null;
      state.proposal = null;
      state.targetId = config.targets[0]?.id || null;
      toast(L('규칙을 처음 상태로 되돌렸어요. 다운로드·문서 폴더부터 다시 설정해 보세요.', 'Rules are back to the defaults. Start again from Downloads and Documents.'), 5000);
      if (state.targetId) await selectTarget(state.targetId);
      renderTargets();
      renderRules();
    } catch (err) {
      toast(L(`초기화하지 못했어요: ${err.message}`, `Couldn't reset: ${err.message}`), 6000);
    }
  }

  root.SmashApp = { renderTargets, selectTarget, addTarget, addTargetPath, removeTarget, resetRules, switchTab };
  root.SmashEvents.bind();
  init();
})(window);
