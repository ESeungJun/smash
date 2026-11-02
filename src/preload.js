'use strict';
const { contextBridge, ipcRenderer } = require('electron');

const call = async (channel, ...args) => {
  const res = await ipcRenderer.invoke(channel, ...args);
  if (!res.ok) throw new Error(res.error);
  return res.data;
};

const on = (channel) => (fn) => {
  const listener = (_e, payload) => fn(payload);
  ipcRenderer.on(channel, listener);
  return () => ipcRenderer.removeListener(channel, listener);
};

contextBridge.exposeInMainWorld('tidy', {
  getConfig: () => call('config:get'),
  saveConfig: (cfg) => call('config:save', cfg),
  scan: (targetId) => call('scan', targetId),
  folders: () => call('folders'),
  testRule: (targetId, rule) => call('rule:test', targetId, rule),
  ruleFromText: (targetId, text) => call('rule:fromText', targetId, text),
  apply: (items) => call('apply', items),
  undo: () => call('undo'),
  history: () => call('history'),
  suggest: (targetId, unmatched) => call('ai:suggest', targetId, unmatched),
  integrationStatus: () => call('integration:status'),
  registerClaudeCode: (locationIds) => call('integration:claudeCode', locationIds),
  registerClaudeDesktop: () => call('integration:claudeDesktop'),
  pickFolder: (defaultPath) => call('dialog:folder', defaultPath),
  showInFolder: (p) => call('shell:show', p),
  openUrl: (url) => call('shell:openUrl', url),
  onAutoRan: on('auto:ran'),
  onTab: on('ui:tab'),
  onConfigChanged: on('config:changed'),
});
