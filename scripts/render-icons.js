'use strict';
// assets/*.svg → build/icon.png(1024), assets/tray*.png 를 만든다.
// 실행: npx electron scripts/render-icons.js
const fs = require('fs');
const path = require('path');
const { app, BrowserWindow } = require('electron');

const root = path.join(__dirname, '..');
// 인자로 [svg out size]...를 주면 그것만 렌더링한다(시안 비교용).
const argJobs = [];
for (let i = 2; i + 2 < process.argv.length + 1 && process.argv[i]?.endsWith('.svg'); i += 3) argJobs.push({ svg: process.argv[i], out: process.argv[i + 1], size: Number(process.argv[i + 2]) });
const defaultJobs = [
  { svg: 'assets/icon.svg', out: 'build/icon.png', size: 1024 },
  { svg: 'assets/icon.svg', out: 'assets/icon-256.png', size: 256 },
  { svg: 'assets/tray.svg', out: 'assets/trayTemplate.png', size: 18 },
  { svg: 'assets/tray.svg', out: 'assets/trayTemplate@2x.png', size: 36 },
  { svg: 'assets/tray-color.svg', out: 'assets/tray-color.png', size: 32 },
];

async function render(win, { svg, out, size }) {
  const markup = fs.readFileSync(path.resolve(root, svg), 'utf8');
  const html = `<html><body style="margin:0;background:transparent">${markup.replace(
    /<svg /,
    `<svg style="width:${size}px;height:${size}px;display:block" `,
  )}</body></html>`;
  win.setContentSize(size, size);
  await win.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(html)}`);
  await new Promise((r) => setTimeout(r, 300));
  const img = await win.webContents.capturePage({ x: 0, y: 0, width: size, height: size });
  const file = path.resolve(root, out);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, img.resize({ width: size, height: size }).toPNG());
  console.log('wrote', out);
}

const jobs = argJobs.length ? argJobs : defaultJobs;

app.whenReady().then(async () => {
  const win = new BrowserWindow({ show: false, transparent: true, frame: false, useContentSize: true, webPreferences: { offscreen: true } });
  win.webContents.setZoomFactor(1);
  for (const job of jobs) await render(win, job);
  app.quit();
});
