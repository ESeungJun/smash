'use strict';
// Tauri 가 띄울 화면 파일을 dist-web/ 에 모은다.
// 화면은 src/renderer/*.html 이고 아이콘을 ../../assets/ 로 부르므로, 같은 상대 위치가 되게 renderer/ 와 assets/ 를 나란히 둔다.
const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..');
const out = path.join(root, 'dist-web');

fs.rmSync(out, { recursive: true, force: true });
fs.mkdirSync(out, { recursive: true });
fs.cpSync(path.join(root, 'src', 'renderer'), path.join(out, 'renderer'), { recursive: true });
fs.mkdirSync(path.join(out, 'assets'));
for (const name of fs.readdirSync(path.join(root, 'assets'))) {
  if (name.endsWith('.png')) fs.copyFileSync(path.join(root, 'assets', name), path.join(out, 'assets', name));
}
// 루트에서 열려도 메인 화면으로 가게 한다.
fs.writeFileSync(path.join(out, 'index.html'), '<!doctype html><meta charset="utf-8"><meta http-equiv="refresh" content="0; url=renderer/index.html">\n');
console.log(`dist-web: ${fs.readdirSync(path.join(out, 'renderer')).length} renderer files`);
