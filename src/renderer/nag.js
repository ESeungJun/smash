'use strict';
// 앱이 ?title=…&sub=…&lang=… 로 넘긴 문구를 넣는다. textContent 라 HTML 로 해석되지 않는다.
const params = new URLSearchParams(location.search);
if (params.get('lang') === 'en') document.documentElement.lang = 'en';
const title = params.get('title') || (params.get('lang') === 'en' ? 'Ugh!! Didn’t I tell you to clean your folders!!' : '');
if (title) document.getElementById('title').textContent = title;
document.getElementById('sub').textContent = params.get('sub') || '';

const banner = document.querySelector('.banner');
const invoke = (cmd) => window.__TAURI__?.core.invoke(cmd);

// 3.8초 보여 주고 위로 올라가며 닫는다. 마우스를 올리면 멈추고, 떼면 다시 센다(읽다가 놓치지 않게).
const STAY_MS = 3800;
const LEAVE_MS = 600;
let timer = null;
function leave() {
  banner.classList.add('leaving');
  setTimeout(() => invoke('nag_close'), LEAVE_MS);
}
function arm() {
  clearTimeout(timer);
  timer = setTimeout(leave, STAY_MS);
}
banner.addEventListener('mouseenter', () => clearTimeout(timer));
banner.addEventListener('mouseleave', arm);
arm();

// 누르면 앱이 정리한 폴더(또는 미리보기·기록)를 열고 배너를 닫는다
banner.addEventListener('click', (e) => {
  e.preventDefault();
  clearTimeout(timer);
  invoke('nag_open');
});
