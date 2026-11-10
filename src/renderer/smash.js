'use strict';
// 앱이 ?lang=…&level=… 로 넘긴 언어와 세기(파일 수에 따라 tap·slap·wham)에 맞춘다.
// 소리는 메인 창이 낸다(사용자 동작이 있었던 창이라 소리가 막히지 않는다).
const params = new URLSearchParams(location.search);
const en = params.get('lang') === 'en';
const level = ['tap', 'slap', 'wham'].includes(params.get('level')) ? params.get('level') : 'slap';
const WORDS = {
  tap: ['톡', 'TAP'],
  slap: ['찰싹!!', 'SMACK!!'],
  wham: ['철썩!!!', 'WHAM!!!'],
};
const word = document.querySelector('.kwang');
document.body.classList.add(`lv-${level}`);
word.textContent = WORDS[level][en ? 1 : 0];
if (en) {
  document.documentElement.lang = 'en';
  word.classList.add('en');
}

// 헛스윙: 하나도 못 치웠을 때 앱이 부른다. 글자만 바꾼다.
window.smashMiss = () => {
  word.textContent = en ? 'Huh?' : '어라?';
  document.body.classList.add('miss');
};
