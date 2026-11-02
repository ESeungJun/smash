'use strict';
// 화면 언어. 한국어와 영어 두 가지라 문장마다 L('한국어', 'English')로 바로 고른다.
// 설정값은 'system' | 'ko' | 'en'. system이면 OS 언어가 한국어일 때만 한국어.

let current = null;
let osLocale = null;

// Electron은 app.getLocale()을 넘겨준다. MCP 서버처럼 Node로만 돌 때는 OS 언어를 직접 읽는다.
// macOS에서 Node의 Intl·LANG은 화면 언어와 다를 때가 많아서(en-US) 기본 언어 목록을 본다.
function readOsLocale() {
  if (osLocale !== null) return osLocale;
  osLocale = '';
  if (process.platform === 'darwin') {
    try {
      const out = require('child_process').execFileSync('defaults', ['read', '-g', 'AppleLanguages'], { encoding: 'utf8', timeout: 2000 });
      osLocale = (out.match(/"?([A-Za-z]{2}[^",\s]*)"?/) || [])[1] || '';
    } catch {
      /* 못 읽으면 Intl로 */
    }
  }
  if (!osLocale) osLocale = Intl.DateTimeFormat().resolvedOptions().locale || process.env.LANG || '';
  return osLocale;
}

function systemLang(locale) {
  return /^ko/i.test(locale || readOsLocale()) ? 'ko' : 'en';
}

function resolveLang(setting, locale) {
  return setting === 'ko' || setting === 'en' ? setting : systemLang(locale);
}

function setLang(lang) {
  current = lang === 'ko' ? 'ko' : 'en';
}

function getLang() {
  return current || systemLang();
}

const L = (ko, en) => (getLang() === 'ko' ? ko : en);

module.exports = { L, getLang, setLang, resolveLang, systemLang };
