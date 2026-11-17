'use strict';
// 본 화면 공통 도구: DOM 찾기, 화면 언어, HTML 이스케이프, 토스트, 크기 표기, 아이콘·스위치 HTML.
// 상태를 갖지 않는다(화면 언어 하나만 기억한다). state.js·tabs/*.js·events.js·app.js 가 window.SmashUI 로 쓴다.
(function (root) {
  const $ = (sel) => document.querySelector(sel);

  // 화면 언어. main이 설정(system이면 OS 언어)을 보고 정해서 config:get으로 넘겨준다. app.js 의 init 이 setLang 으로 넣는다.
  let lang = 'ko';
  const setLang = (v) => {
    lang = v;
  };
  const getLang = () => lang;
  const L = (ko, en) => (lang === 'ko' ? ko : en);

  const esc = (s) =>
    String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

  const reducedMotion = () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

  let toastTimer;
  function toast(msg, ms = 2800) {
    const el = $('#toast');
    el.textContent = msg;
    el.classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => el.classList.remove('show'), ms);
  }

  // 1024 단위. 기록 탭의 주간 합계는 SmashLogic.fmtBytes(1000 단위)를 쓴다.
  function fmtSize(n) {
    if (n >= 1 << 30) return `${(n / (1 << 30)).toFixed(1)}GB`;
    if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(1)}MB`;
    return `${Math.max(1, Math.round(n / 1024))}KB`;
  }

  // 16px 선 아이콘. 색은 currentColor를 따른다.
  const ICONS = {
    folder: '<path d="M2 4.5c0-.6.4-1 1-1h3.2l1.5 1.6H13c.6 0 1 .4 1 1V12c0 .6-.4 1-1 1H3c-.6 0-1-.4-1-1z"/>',
    file: '<path d="M4 2h5l3 3v8.5c0 .3-.2.5-.5.5h-7a.5.5 0 0 1-.5-.5v-11c0-.3.2-.5.5-.5z"/><path d="M9 2v3h3"/>',
    trash: '<path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.6c0 .5.4.9.9.9h4c.5 0 .9-.4.9-.9l.6-8.6"/>',
  };
  const ico = (name) =>
    `<svg class="ico" viewBox="0 0 16 16" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round" stroke-linecap="round" aria-hidden="true">${ICONS[name]}</svg>`;

  const switchHtml = (attr, on) => `<label class="switch"><input type="checkbox" ${attr} ${on ? 'checked' : ''} /><span></span></label>`;
  const dotLine = (on, onText, offText) => `<div class="status-line"><span class="dot ${on ? 'on' : 'off'}"></span>${on ? onText : offText}</div>`;

  root.SmashUI = { $, setLang, getLang, L, esc, reducedMotion, toast, fmtSize, ico, switchHtml, dotLine };
})(window);
