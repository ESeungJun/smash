'use strict';
// 설정 파일 위치·기본 규칙·읽고 쓰기. GUI와 MCP 서버가 같은 파일을 쓴다.
const fs = require('fs');
const os = require('os');
const path = require('path');
const { L } = require('./lang');

const APP_DIR_NAME = 'Smash';

function configDir() {
  if (process.env.SMASH_HOME) return process.env.SMASH_HOME;
  const home = os.homedir();
  if (process.platform === 'darwin') return path.join(home, 'Library', 'Application Support', APP_DIR_NAME);
  if (process.platform === 'win32') return path.join(process.env.APPDATA || path.join(home, 'AppData', 'Roaming'), APP_DIR_NAME);
  return path.join(process.env.XDG_CONFIG_HOME || path.join(home, '.config'), 'smash');
}

const configPath = () => path.join(configDir(), 'config.json');
const historyPath = () => path.join(configDir(), 'history.json');

function expandHome(p) {
  if (!p) return p;
  if (p === '~') return os.homedir();
  if (p.startsWith('~/') || p.startsWith('~\\')) return path.join(os.homedir(), p.slice(2));
  return p;
}

function contractHome(p) {
  const home = os.homedir();
  if (p === home) return '~';
  if (p.startsWith(home + path.sep)) return '~' + p.slice(home.length).split(path.sep).join('/');
  return p;
}

// 규칙 화면의 '파일 종류' 칩. 기본 규칙도 이걸로 만든다. 이름과 폴더는 언어마다 다르다(localCategories).
const CATEGORIES = [
  { key: 'image', label: '사진·이미지', labelEn: 'Images', folder: '사진·이미지', folderEn: 'Images', extensions: ['jpg', 'jpeg', 'png', 'gif', 'heic', 'webp', 'tiff', 'bmp', 'svg'] },
  { key: 'video', label: '동영상', labelEn: 'Video', folder: '미디어', folderEn: 'Media', extensions: ['mp4', 'mov', 'avi', 'mkv', 'webm', 'm4v'] },
  { key: 'audio', label: '음악·녹음', labelEn: 'Audio', folder: '미디어', folderEn: 'Media', extensions: ['mp3', 'm4a', 'wav', 'flac', 'aac', 'ogg'] },
  { key: 'doc', label: '문서', labelEn: 'Documents', folder: '자료', folderEn: 'Files', extensions: ['pdf', 'hwp', 'hwpx', 'doc', 'docx', 'txt', 'md', 'rtf', 'pages', 'html'] },
  { key: 'sheet', label: '표·슬라이드', labelEn: 'Sheets & slides', folder: '자료', folderEn: 'Files', extensions: ['xls', 'xlsx', 'csv', 'numbers', 'ppt', 'pptx', 'key'] },
  { key: 'archive', label: '압축 파일', labelEn: 'Archives', folder: '자료', folderEn: 'Files', extensions: ['zip', '7z', 'rar', 'tar', 'gz'] },
  { key: 'installer', label: '설치 파일', labelEn: 'Installers', folder: '', folderEn: '', extensions: ['dmg', 'pkg', 'exe', 'msi'] },
  { key: 'dev', label: '폰트·개발', labelEn: 'Fonts & dev', folder: '개발리소스', folderEn: 'Dev Resources', extensions: ['ttf', 'otf', 'woff', 'woff2', 'json', 'yaml', 'yml', 'log', 'xml'] },
];

const localCategories = () =>
  CATEGORIES.map(({ labelEn, folderEn, ...c }) => ({ ...c, label: L(c.label, labelEn), folder: L(c.folder, folderEn) }));

let idSeq = 0;
function newId(prefix) {
  idSeq += 1;
  return `${prefix}_${Date.now().toString(36)}${idSeq.toString(36)}`;
}

function rule(name, extensions, dest, extra = {}) {
  return {
    id: newId('r'),
    name,
    enabled: true,
    match: { extensions, nameIncludes: [], olderThanDays: 0, ...(extra.match || {}) },
    action: extra.action || 'move',
    dest: extra.action === 'trash' ? '' : dest,
  };
}

// 문서 폴더를 기준으로 정리한다. 다운로드에 들어온 파일도 문서 폴더의 같은 분류로 보낸다.
function categoryRules(destBase) {
  const cat = (key) => localCategories().find((c) => c.key === key);
  const media = [...cat('video').extensions, ...cat('audio').extensions];
  const docs = [...cat('doc').extensions, ...cat('sheet').extensions, ...cat('archive').extensions];
  const to = (folder) => (destBase ? `${destBase}/${folder}` : folder);
  return [
    rule(L('사진·이미지', 'Images'), cat('image').extensions, to(cat('image').folder)),
    rule(L('동영상·음악·녹음', 'Video & audio'), media, to(cat('video').folder)),
    rule(L('문서·표·압축 파일', 'Documents, sheets & archives'), docs, to(cat('doc').folder)),
    rule(L('폰트·개발 리소스', 'Fonts & dev resources'), cat('dev').extensions, to(cat('dev').folder)),
  ];
}

function defaultConfig() {
  return {
    version: 2,
    targets: [
      {
        id: 'downloads',
        path: '~/Downloads',
        auto: false,
        rules: [
          rule(L('오래된 설치 파일', 'Old installers'), CATEGORIES.find((c) => c.key === 'installer').extensions, '', { action: 'trash', match: { olderThanDays: 7 } }),
          ...categoryRules('~/Documents'),
        ],
      },
      { id: 'documents', path: '~/Documents', auto: false, rules: categoryRules('') },
    ],
    skip: {
      hidden: true,
      patterns: ['.localized', 'desktop.ini', 'Thumbs.db', '*.crdownload', '*.download', '*.part', '*.tmp', '.정리기록*'],
    },
    ai: { model: 'sonnet' },
    settings: { autoDelaySec: 10, notify: true, openAtLogin: false, smash: true, openAfter: true, language: 'system' },
  };
}

function normalize(cfg) {
  const base = defaultConfig();
  const out = { ...base, ...cfg };
  out.skip = { ...base.skip, ...(cfg.skip || {}) };
  out.ai = { ...base.ai, ...(cfg.ai || {}) };
  out.settings = { ...base.settings, ...(cfg.settings || {}) };
  out.settings.autoDelaySec = Math.min(3600, Math.max(3, Number(out.settings.autoDelaySec) || base.settings.autoDelaySec));
  out.settings.smash = out.settings.smash !== false;
  out.settings.openAfter = out.settings.openAfter !== false;
  if (!['system', 'ko', 'en'].includes(out.settings.language)) out.settings.language = 'system';
  out.targets = (Array.isArray(cfg.targets) ? cfg.targets : base.targets).map((t) => ({
    id: t.id || newId('t'),
    path: String(t.path || ''),
    auto: t.auto === true,
    rules: (Array.isArray(t.rules) ? t.rules : []).map((r) => ({
      id: r.id || newId('r'),
      name: String(r.name || L('이름 없는 규칙', 'Untitled rule')),
      enabled: r.enabled !== false,
      match: {
        extensions: (r.match?.extensions || []).map((e) => String(e).toLowerCase().replace(/^\./, '')).filter(Boolean),
        nameIncludes: (r.match?.nameIncludes || []).map(String).filter(Boolean),
        olderThanDays: Math.max(0, Number(r.match?.olderThanDays) || 0),
      },
      action: r.action === 'trash' ? 'trash' : 'move',
      dest: r.action === 'trash' ? '' : String(r.dest || ''),
    })),
  }));
  return out;
}

function writeJsonAtomic(file, data) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(data, null, 2));
  fs.renameSync(tmp, file);
}

function loadConfig() {
  const file = configPath();
  if (!fs.existsSync(file)) {
    const cfg = defaultConfig();
    writeJsonAtomic(file, cfg);
    return cfg;
  }
  return normalize(JSON.parse(fs.readFileSync(file, 'utf8')));
}

function saveConfig(cfg) {
  const clean = normalize(cfg);
  writeJsonAtomic(configPath(), clean);
  return clean;
}

module.exports = {
  CATEGORIES,
  localCategories,
  configDir,
  configPath,
  historyPath,
  expandHome,
  contractHome,
  newId,
  defaultConfig,
  normalize,
  loadConfig,
  saveConfig,
  writeJsonAtomic,
};
