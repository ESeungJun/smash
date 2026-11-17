'use strict';
/* global tidy, SmashLogic, SmashSound, SmashUI, SmashState */

const { $, L, esc, reducedMotion, toast, fmtSize, ico, switchHtml, dotLine } = SmashUI;
const { state, adoptConfig, saveConfig, baseName, dirOf, expand, contract, currentTarget, targetName, cliReady, linkBase, destText, destInside } =
  SmashState;
const { refreshIntegration, renderUpdate, checkUpdate, renderSettings, askAuto, confirmAuto, register } = SmashSettings;
const { syncRemoveButton, destPreviewHtml, renderEditor, renderRules, openEditor, addTag, saveDraft, makeRuleFromText, saveProposal, pickDest, loadFolders, handleDraftClick } = SmashRules;
const { renderHistory, doUndo } = SmashHistory;
const { doScan, allPlanItems, syncFolderChecks, renderPreview, renderResult, renderIntro, aiGroupMedia, updateApplyButton, aiSuggest, doApply, onHit, onProgress } = SmashPreview;
// 저장할 때마다 사이드바 폴더 목록을 다시 그린다
SmashState.onSaved(() => renderTargets());

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

// ---------- 미리보기 (트리) ----------

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

// ---------- 규칙 ----------

// ---------- 기록 ----------

// ---------- 설정 (자동 정리 · 효과 · Claude) ----------

// ---------- 이벤트 ----------

document.addEventListener('click', async (e) => {
  const el = e.target.closest('button, [data-tab]');
  if (!el) return;
  if (el.dataset.tab) return switchTab(el.dataset.tab);
  if (el.dataset.goto) return switchTab(el.dataset.goto);
  if (el.dataset.url) return tidy.openUrl(el.dataset.url);
  if (el.classList.contains('target-btn')) return selectTarget(el.dataset.id);
  if (el.dataset.removeTarget) return removeTarget(el.dataset.removeTarget);
  if (el.dataset.gotoTarget) return selectTarget(el.dataset.gotoTarget);
  if (el.dataset.undoRun) {
    e.preventDefault(); // 줄(details)이 같이 펼쳐지지 않게
    return doUndo(el.dataset.undoRun);
  }
  if (el.dataset.addTargetPath) return addTargetPath(el.dataset.addTargetPath);
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
    case 'remove-target': return removeTarget(state.targetId);
    case 'ai-group': return aiGroupMedia();
    case 'proposal-save': return saveProposal();
    case 'proposal-cancel':
      state.proposal = null;
      return renderRules();
    case 'add-rule': return openEditor(null);
    case 'nl-make': return makeRuleFromText();
    case 'cancel-rule':
      state.draft = null;
      return renderRules();
    case 'save-rule': return saveDraft();
    case 'pick-dest': return pickDest();
    case 'undo': return doUndo();
    case 'result-close':
      e.preventDefault();
      state.lastResult = null;
      return renderResult();
    case 'intro-ok':
      state.config.settings.introSeen = true;
      renderIntro();
      return saveConfig();
    case 'dest-add-target': {
      const t = currentTarget();
      return addTargetPath(SmashLogic.resolvePath(t.path, state.draft.dest.trim(), state.home, state.sep)).then(() => renderEditor());
    }
    case 'auto-confirm-on': return confirmAuto();
    case 'auto-confirm-cancel':
      state.autoConfirm = null;
      return renderSettings();
    case 'reg-code': return register('code');
    case 'reg-desktop': return register('desktop');
    case 'update-check': return checkUpdate();
    case 'update-install':
      el.disabled = true;
      return tidy.installUpdate();
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
  if (el.id === 'd-ext' || el.id === 'd-word' || el.id === 'd-from') {
    const kind = { 'd-ext': 'ext', 'd-word': 'word', 'd-from': 'from' }[el.id];
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
  if (el.dataset.proposal !== undefined && state.proposal) {
    const i = Number(el.dataset.proposal);
    if (el.checked) state.proposal.picked.add(i);
    else state.proposal.picked.delete(i);
    const btn = $('#proposal-save');
    btn.disabled = !state.proposal.picked.size;
    btn.textContent = L(`고른 ${state.proposal.picked.size}개 저장`, `Save ${state.proposal.picked.size} selected`);
    return;
  }
  if (el.id === 't-recursive') {
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
  if (el.id === 't-link') {
    const t = currentTarget();
    t.link = el.value;
    await saveConfig();
    renderRules();
    return doScan();
  }
  if (el.id === 'set-vision') {
    state.config.ai = { ...(state.config.ai || {}), vision: el.checked };
    await saveConfig();
    if (state.scan) renderPreview();
    return toast(
      el.checked
        ? L('켰어요. 미리보기에서 "내용 보고 묶기"를 누를 때만 썸네일을 보내요.', 'On. Thumbnails are sent only when you press "Group by content" in the preview.')
        : L('껐어요. 사진 내용은 Claude에 보내지 않아요.', 'Off. No photo content goes to Claude.'),
      4000,
    );
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
    if (el.checked) {
      // 바로 켜지 않는다. 지금 쌓인 파일을 어떻게 치울지 먼저 보여 준다
      el.checked = false;
      return askAuto(t.id);
    }
    state.autoConfirm = null;
    t.auto = false;
    await saveConfig();
    renderSettings();
    return toast(L(`${targetName(t)} 자동 정리를 껐어요.`, `Auto-tidy off for ${targetName(t)}.`));
  }
  if (el.id === 'auto-trash' && state.autoConfirm) {
    state.autoConfirm.autoTrash = el.checked;
    return renderSettings();
  }
  if (el.id === 'auto-delay') {
    state.config.settings.autoDelaySec = Number(el.value);
    return saveConfig();
  }
  const toggles = { 'set-notify': 'notify', 'set-login': 'openAtLogin', 'set-smash': 'smash', 'set-open': 'openAfter', 'set-sound': 'sound', 'set-nag': 'nag', 'set-weekly': 'weekly' };
  if (toggles[el.id]) {
    state.config.settings[toggles[el.id]] = el.checked;
    // 소리를 켜면 어떤 소리인지 한 번 들려준다
    if (el.id === 'set-sound' && el.checked) SmashSound.play('slap');
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
tidy.onUpdate((u) => {
  state.update = u;
  renderUpdate();
});
// 트레이나 다른 프로그램(Claude 의 MCP 서버)이 설정을 바꿨다. 편집 중인 규칙은 건드리지 않는다.
tidy.onConfigChanged(async () => {
  const { config, rev } = await tidy.getConfig();
  if (rev === state.rev) return;
  adoptConfig(config, rev);
  if (!config.targets.some((t) => t.id === state.targetId)) state.targetId = config.targets[0]?.id || null;
  renderTargets();
  if ($('#tab-settings').classList.contains('active')) renderSettings();
  if (!state.draft && !state.proposal) renderRules();
  if (!state.busy && !state.aiItems.length) await doScan();
});
tidy.onHit(onHit);
tidy.onProgress(onProgress);
// 트레이에서 자동 정리를 켜려 했다: 설정 탭에서 먼저 확인받는다
tidy.onAutoConfirm((id) => {
  switchTab('settings');
  askAuto(id);
});
// 잔소리 배너를 눌렀다: 그 폴더의 미리보기로
tidy.onTarget((id) => {
  if (state.config.targets.some((t) => t.id === id)) selectTarget(id);
});
// 브라우저는 사용자 동작 뒤에만 소리를 허용한다. 첫 클릭 때 깨워 둔다.
document.addEventListener('pointerdown', () => SmashSound.warm(), { once: true });

// index.html에 적힌 고정 문구(한국어)를 화면 언어에 맞춘다.
function translateStatic() {
  document.documentElement.lang = SmashUI.getLang();
  if (SmashUI.getLang() === 'ko') return;
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
  set('#undo', 'Undo last run');
}

(async function init() {
  const { config, configPath, home, sep, categories, platform, lang, rev, canRestoreTrash } = await tidy.getConfig();
  Object.assign(state, { configPath, home, sep, categories, platform, canRestoreTrash: canRestoreTrash !== false });
  adoptConfig(config, rev);
  SmashUI.setLang(lang);
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
})();
