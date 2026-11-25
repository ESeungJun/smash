'use strict';
/* global tidy, SmashLogic */
// 본 화면의 상태 하나(state)와 설정 읽기·저장, 그리고 state 의 홈·구분자·정리 대상 폴더를 쓰는 경로 도우미.
// 경로 계산 자체는 logic.js(SmashLogic)에 있고 여기서는 state 값을 넣어 부르기만 한다.
// 모든 탭 모듈과 events.js·app.js 가 window.SmashState 로 쓴다.
(function (root) {
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
    proposal: null, // Claude가 만든 규칙 세트 { summary, changes, picked:Set }
    removeArmed: null, // 사이드바에서 빼기 확인을 기다리는 폴더 id
    resetArmed: false, // 규칙 초기화 확인을 기다리는 중(#11)
    folders: [],
    integration: null,
    update: null,
    rev: '', // 마지막으로 읽거나 저장한 설정 파일의 상태
    base: null, // 그때의 설정(다른 프로그램이 바꾼 것과 합칠 때 기준)
    canRestoreTrash: true,
    lastResult: null, // 방금 정리한 결과 띠 { ok, failed, historyError, moved, destLine, runId }. 되돌리거나 다른 폴더를 고르면 비운다
    autoConfirm: null, // 자동 정리를 켜기 전 확인 중인 폴더 { id, scan, autoTrash }
    autoCollapse: false,
  };

  // ---------- 설정 읽기·저장 ----------

  function adoptConfig(config, rev) {
    state.config = config;
    state.rev = rev;
    state.base = JSON.parse(JSON.stringify(config));
  }

  // 저장이 끝날 때마다 부를 함수. app.js 가 사이드바 다시 그리기를 넣는다(이 파일이 사이드바를 몰라도 되게).
  let afterSave = () => {};
  const onSaved = (fn) => {
    afterSave = fn;
  };

  // 저장 직전에 디스크를 다시 본다. 그 사이 다른 프로그램(Claude 의 MCP 서버)이 규칙을 넣었으면 합쳐서 저장한다.
  async function saveConfig() {
    try {
      const disk = await tidy.getConfig();
      if (disk.rev !== state.rev) state.config = SmashLogic.mergeConfig(state.base, state.config, disk.config);
    } catch {
      // 못 읽으면 화면 것을 그대로 저장한다
    }
    const saved = await tidy.saveConfig(state.config);
    adoptConfig(saved.config, saved.rev);
    afterSave();
  }

  // ---------- 경로·폴더 도우미 ----------

  const baseName = SmashLogic.baseName;
  const dirOf = (p) => p.slice(0, p.lastIndexOf(state.sep));
  const expand = (p) => SmashLogic.toNativePath(p, state.home, state.sep);
  const contract = (p) => SmashLogic.contractPath(p, state.home, state.sep);

  const currentTarget = () => state.config.targets.find((t) => t.id === state.targetId);
  const targetName = (t) => baseName(t.path);
  const cliReady = () => state.integration?.cli?.installed && !state.integration.cli.error;
  const linkBase = (t) => (t?.link ? state.config.targets.find((x) => x.id === t.link) : null);

  /** 규칙 목적지를 화면에 보일 이름으로. owner 가 없으면 지금 보는 폴더 기준. */
  function destText(dest, owner) {
    return SmashLogic.destLabel(dest, owner || currentTarget(), state.config.targets, state.home, state.sep);
  }

  /** 규칙 목적지가 정리할 폴더 안인지 */
  function destInside(dest, owner) {
    const t = owner || currentTarget();
    return !t || SmashLogic.destInsideTargets(dest, t.path, state.config.targets, state.home, state.sep);
  }

  // 플랫폼마다 다른 이름(#10): "Finder에서 보기" 버튼, 휴지통
  function revealLabel() {
    const { L } = root.SmashUI;
    if (state.platform === 'darwin') return L('Finder에서 보기', 'Show in Finder');
    if (state.platform === 'win32') return L('탐색기에서 보기', 'Show in Explorer');
    return L('폴더에서 보기', 'Show in folder');
  }
  function trashLabel() {
    const { L } = root.SmashUI;
    return L('휴지통', state.platform === 'win32' ? 'Recycle Bin' : 'Trash');
  }

  root.SmashState = {
    state,
    revealLabel,
    trashLabel,
    adoptConfig,
    onSaved,
    saveConfig,
    baseName,
    dirOf,
    expand,
    contract,
    currentTarget,
    targetName,
    cliReady,
    linkBase,
    destText,
    destInside,
  };
})(window);
