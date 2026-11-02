'use strict';
// main.js가 ?sub=... 로 넘긴 한 줄 설명을 넣는다. textContent라 HTML로 해석되지 않는다.
document.getElementById('sub').textContent = new URLSearchParams(location.search).get('sub') || '';
