'use strict';
/* global tidy, SmashLogic, SmashSound */
// 본 화면의 이벤트 연결. 문서 전체에 거는 위임 처리기(click·mousedown·keydown·input·change)와 Tauri 알림(tidy.on*)을 건다.
// 여기서는 어느 요소인지만 보고 탭 모듈(SmashPreview·SmashRules·SmashHistory·SmashSettings)이나 SmashApp 으로 넘긴다.
// 분기 순서가 곧 우선순위다(예: 편집기가 열려 있으면 편집기 버튼이 규칙 카드 버튼보다 먼저). 순서를 바꾸지 않는다.
// app.js 가 시작할 때 init 보다 먼저 bind() 를 한 번 부른다.
// SmashApp 은 bind() 안에서 꺼내므로, app.js 가 root.SmashApp 을 넣은 뒤에 bind() 를 불러야 한다.
(function (root) {
  function bind() {
    const { $, L, toast } = root.SmashUI;
    const { state, adoptConfig, currentTarget } = root.SmashState;
    const Preview = root.SmashPreview;
    const Rules = root.SmashRules;
    const History = root.SmashHistory;
    const Settings = root.SmashSettings;
    const App = root.SmashApp;

    document.addEventListener('click', async (e) => {
      const el = e.target.closest('button, [data-tab]');
      if (!el) return;
      if (el.dataset.tab) return App.switchTab(el.dataset.tab);
      if (el.dataset.goto) return App.switchTab(el.dataset.goto);
      if (el.dataset.url) return tidy.openUrl(el.dataset.url);
      if (el.classList.contains('target-btn')) return App.selectTarget(el.dataset.id);
      if (el.dataset.removeTarget) return App.removeTarget(el.dataset.removeTarget);
      if (el.dataset.gotoTarget) return App.selectTarget(el.dataset.gotoTarget);
      if (el.dataset.undoRun) {
        e.preventDefault(); // 줄(details)이 같이 펼쳐지지 않게
        return History.doUndo(el.dataset.undoRun);
      }
      if (el.dataset.addTargetPath) return App.addTargetPath(el.dataset.addTargetPath);
      if (el.dataset.historyView) return History.setView(el.dataset.historyView);
      if (el.dataset.showPath) {
        e.preventDefault(); // 줄(details)이 같이 접히지 않게
        return tidy.showInFolder(el.dataset.showPath).catch((err) => toast(L(`열지 못했어요: ${err.message}`, `Couldn't open it: ${err.message}`)));
      }
      if (el.dataset.collapse) return Preview.toggleCollapse(el.dataset.collapse);
      if (state.draft && Rules.handleDraftClick(el)) return;
      const t = currentTarget();
      if (el.dataset.ruleEdit) return Rules.editRule(t, el.dataset.ruleEdit);
      if (el.dataset.ruleDelete) return Rules.deleteRule(t, el.dataset.ruleDelete);
      if (el.dataset.ruleMove) return Rules.moveRule(t, el.dataset.ruleMove, el.dataset.dir);
      switch (el.id) {
        case 'scan': return Preview.doScan();
        case 'apply': return Preview.doApply();
        case 'ai-suggest': return Preview.aiSuggest();
        case 'toggle-all': return Preview.toggleAll();
        case 'add-target': return App.addTarget();
        case 'remove-target': return App.removeTarget(state.targetId);
        case 'reset-rules': return App.resetRules();
        case 'ai-group': return Preview.aiGroupMedia();
        case 'proposal-save': return Rules.saveProposal();
        case 'proposal-cancel': return Rules.cancelProposal();
        case 'add-rule': return Rules.openEditor(null);
        case 'nl-make': return Rules.makeRuleFromText();
        case 'cancel-rule': return Rules.cancelDraft();
        case 'save-rule': return Rules.saveDraft();
        case 'pick-dest': return Rules.pickDest();
        case 'undo': return History.doUndo();
        case 'result-close':
          e.preventDefault();
          return Preview.closeResult();
        case 'intro-ok': return Preview.dismissIntro();
        case 'dest-add-target': {
          // 편집기 목적지가 정리할 폴더 밖일 때: 그 폴더를 정리할 폴더로 더하고 편집기를 다시 그린다
          const t = currentTarget();
          return App.addTargetPath(SmashLogic.resolvePath(t.path, state.draft.dest.trim(), state.home, state.sep)).then(() => Rules.renderEditor());
        }
        case 'auto-confirm-on': return Settings.confirmAuto();
        case 'auto-confirm-cancel': return Settings.cancelAutoConfirm();
        case 'reg-code': return Settings.register('code');
        case 'reg-desktop': return Settings.register('desktop');
        case 'update-check': return Settings.checkUpdate();
        case 'update-install': return Settings.installUpdate(el);
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
      if (el.id === 'nl-text' && e.key === 'Enter') return Rules.makeRuleFromText();
      if (Rules.isTagInput(el)) return Rules.tagKeydown(e);
    });

    document.addEventListener('input', (e) => {
      if (e.target.id === 'history-q') return History.search(e.target.value);
      return Rules.draftInput(e);
    });

    document.addEventListener('change', async (e) => {
      const el = e.target;
      if (el.dataset.item) return Preview.checkItem(el);
      if (el.dataset.folder) return Preview.checkFolder(el);
      if (el.dataset.proposal !== undefined && state.proposal) return Rules.pickProposal(el);
      if (el.id === 't-recursive') return Rules.setRecursive(el);
      if (el.id === 't-link') return Rules.setLink(el.value);
      if (el.id === 'set-vision') return Settings.setVision(el);
      if (el.dataset.ruleToggle) return Rules.toggleRule(el.dataset.ruleToggle, el.checked);
      if (el.dataset.auto) return Settings.switchAuto(el);
      if (el.id === 'auto-trash' && state.autoConfirm) return Settings.setAutoTrash(el.checked);
      if (el.id === 'auto-delay') return Settings.setAutoDelay(el.value);
      if (Settings.isToggle(el)) return Settings.setToggle(el);
      if (el.id === 'set-lang') return Settings.setLanguage(el.value);
      if (el.id === 'ai-model') return Settings.setModel(el);
    });

    tidy.onAutoRan(async ({ count }) => {
      if (count) toast(L(`짝! 자동으로 ${count}개 치웠어요.`, `Smack! Auto-tidied ${count} files.`));
      if (!state.busy && !state.aiItems.length) await Preview.doScan();
      if ($('#tab-history').classList.contains('active')) History.renderHistory();
    });
    tidy.onTab((tab) => App.switchTab(tab));
    tidy.onUpdate((u) => {
      state.update = u;
      Settings.renderUpdate();
    });
    // 트레이나 다른 프로그램(Claude 의 MCP 서버)이 설정을 바꿨다. 편집 중인 규칙은 건드리지 않는다.
    tidy.onConfigChanged(async () => {
      const { config, rev } = await tidy.getConfig();
      if (rev === state.rev) return;
      adoptConfig(config, rev);
      if (!config.targets.some((t) => t.id === state.targetId)) state.targetId = config.targets[0]?.id || null;
      App.renderTargets();
      if ($('#tab-settings').classList.contains('active')) Settings.renderSettings();
      if (!state.draft && !state.proposal) Rules.renderRules();
      if (!state.busy && !state.aiItems.length) await Preview.doScan();
    });
    tidy.onHit(Preview.onHit);
    tidy.onProgress(Preview.onProgress);
    // 트레이에서 자동 정리를 켜려 했다: 설정 탭에서 먼저 확인받는다
    tidy.onAutoConfirm((id) => {
      App.switchTab('settings');
      Settings.askAuto(id);
    });
    // 잔소리 배너를 눌렀다: 그 폴더의 미리보기로
    tidy.onTarget((id) => {
      if (state.config.targets.some((t) => t.id === id)) App.selectTarget(id);
    });
    // 브라우저는 사용자 동작 뒤에만 소리를 허용한다. 첫 클릭 때 깨워 둔다.
    document.addEventListener('pointerdown', () => SmashSound.warm(), { once: true });
  }

  root.SmashEvents = { bind };
})(window);
