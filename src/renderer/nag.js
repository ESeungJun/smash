'use strict';
// 앱이 ?sub=...&lang=... 로 넘긴 한 줄 설명과 언어를 넣는다. textContent라 HTML로 해석되지 않는다.
const params = new URLSearchParams(location.search);
if (params.get('lang') === 'en') {
  document.documentElement.lang = 'en';
  document.querySelector('.title').textContent = 'Ugh!! Didn’t I tell you to clean your folders!!';
}
document.getElementById('sub').textContent = params.get('sub') || '';

// 누르면 앱이 정리한 폴더를 열고 배너를 닫는다
document.querySelector('.banner').addEventListener('click', (e) => {
  e.preventDefault();
  window.__TAURI__?.core.invoke('nag_open');
});
