'use strict';
// 화면이 쓰는 window.tidy 를 Tauri 명령으로 잇는다 (Electron 의 preload.js 자리).
// 명령 이름·인자는 src-tauri/src/lib.rs 의 #[tauri::command] 와 맞춘다.
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;

  // Rust 쪽 오류는 문자열로 온다. 화면은 err.message 를 쓰므로 Error 로 바꾼다.
  const call = (cmd, args) =>
    invoke(cmd, args).catch((err) => {
      throw new Error(typeof err === 'string' ? err : err?.message || String(err));
    });

  // listen 은 Promise 로 해제 함수를 주지만, 화면은 바로 해제 함수를 받길 기대한다.
  const on = (event) => (fn) => {
    let off = null;
    let cancelled = false;
    listen(event, (e) => fn(e.payload)).then((unlisten) => {
      if (cancelled) unlisten();
      else off = unlisten;
    });
    return () => {
      cancelled = true;
      off?.();
    };
  };

  window.tidy = {
    getConfig: () => call('config_get'),
    saveConfig: (cfg) => call('config_save', { cfg }),
    scan: (targetId) => call('scan', { targetId }),
    folders: () => call('folders'),
    testRule: (targetId, rule) => call('rule_test', { targetId, rule }),
    ruleFromText: (targetId, text) => call('rule_from_text', { targetId, text: String(text) }),
    apply: (items) => call('apply', { items }),
    undo: () => call('undo'),
    history: () => call('history'),
    suggest: (targetId, unmatched) => call('ai_suggest', { targetId, unmatched }),
    integrationStatus: () => call('integration_status'),
    registerClaudeCode: (locationIds) => call('integration_claude_code', { locationIds: locationIds ?? null }),
    registerClaudeDesktop: () => call('integration_claude_desktop'),
    pickFolder: (defaultPath) => call('dialog_folder', { defaultPath: defaultPath ?? null }),
    showInFolder: (path) => call('shell_show', { path }),
    openUrl: (url) => call('shell_open_url', { url }),
    onAutoRan: on('auto:ran'),
    onTab: on('ui:tab'),
    onConfigChanged: on('config:changed'),
    updateStatus: () => call('update_status'),
    checkUpdate: () => call('update_check'),
    installUpdate: () => call('update_install'),
    onUpdate: on('update:state'),
  };
})();
