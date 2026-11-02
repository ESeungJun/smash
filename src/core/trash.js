'use strict';
// OS 휴지통으로 보내기. 영구 삭제는 하지 않는다.
const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync } = require('child_process');
const { L } = require('./lang');

// NSFileManager를 직접 불러서 Finder 자동화 권한 없이 휴지통으로 보낸다.
const MAC_SCRIPT =
  'function run(argv){ObjC.import("Foundation");' +
  'return $.NSFileManager.defaultManager.trashItemAtURLResultingItemURLError($.NSURL.fileURLWithPath(argv[0]),null,null)}';

const WIN_SCRIPT =
  'Add-Type -AssemblyName Microsoft.VisualBasic;' +
  "[Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($env:SMASH_TRASH,'OnlyErrorDialogs','SendToRecycleBin')";

function trashLinux(file) {
  const base = path.join(process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share'), 'Trash');
  fs.mkdirSync(path.join(base, 'files'), { recursive: true });
  fs.mkdirSync(path.join(base, 'info'), { recursive: true });
  let name = path.basename(file);
  for (let i = 2; fs.existsSync(path.join(base, 'files', name)); i += 1) {
    const ext = path.extname(file);
    name = `${path.basename(file, ext)}.${i}${ext}`;
  }
  const info = `[Trash Info]\nPath=${encodeURI(file)}\nDeletionDate=${new Date().toISOString().slice(0, 19)}\n`;
  fs.writeFileSync(path.join(base, 'info', `${name}.trashinfo`), info);
  fs.renameSync(file, path.join(base, 'files', name));
}

function moveToTrash(file) {
  if (process.platform === 'darwin') {
    const out = execFileSync('osascript', ['-l', 'JavaScript', '-e', MAC_SCRIPT, file], { encoding: 'utf8' }).trim();
    if (out !== 'true' || fs.existsSync(file)) throw new Error(L('휴지통으로 옮기지 못했습니다', 'Could not move to Trash'));
    return;
  }
  if (process.platform === 'win32') {
    execFileSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', WIN_SCRIPT], {
      env: { ...process.env, SMASH_TRASH: file },
      windowsHide: true,
    });
    if (fs.existsSync(file)) throw new Error(L('휴지통으로 옮기지 못했습니다', 'Could not move to Trash'));
    return;
  }
  trashLinux(file);
}

module.exports = { moveToTrash };
