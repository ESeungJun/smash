'use strict';
// 영어 화면이면 의성어를 바꾼다. 글자가 더 길어서 조금 작게 쓴다(smash.css의 .word.en).
if (new URLSearchParams(location.search).get('lang') === 'en') {
  document.documentElement.lang = 'en';
  const word = document.querySelector('.kwang');
  word.textContent = 'SMACK!!';
  word.classList.add('en');
}
