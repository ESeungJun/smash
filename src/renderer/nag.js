'use strict';
// main.js가 ?sub=...&lang=... 로 넘긴 한 줄 설명과 언어를 넣는다. textContent라 HTML로 해석되지 않는다.
const params = new URLSearchParams(location.search);
if (params.get('lang') === 'en') {
  document.documentElement.lang = 'en';
  document.querySelector('.title').textContent = 'Ugh!! Didn’t I tell you to clean your folders!!';
}
document.getElementById('sub').textContent = params.get('sub') || '';
