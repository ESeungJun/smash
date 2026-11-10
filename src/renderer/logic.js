'use strict';
// 화면에서 쓰는 순수 계산. DOM·Tauri 를 쓰지 않아서 `node --test` 로 시험한다(test/logic.test.js).
// 브라우저에서는 window.SmashLogic, Node 에서는 module.exports 로 내보낸다.
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.SmashLogic = api;
})(typeof self !== 'undefined' ? self : this, () => {
  const clone = (v) => JSON.parse(JSON.stringify(v));

  /**
   * 설정 세 벌을 합친다. base 는 화면이 마지막으로 읽은(또는 저장한) 것, mine 은 화면에서 고친 것,
   * theirs 는 지금 디스크에 있는 것(그 사이 MCP 서버가 규칙을 넣었을 수 있다).
   * 화면에서 고친 내용이 이긴다. 디스크에만 새로 생긴 규칙·폴더는 살려서 넣는다.
   * 화면에서 지운 것(base 에는 있고 mine 에 없는 것)은 되살리지 않는다.
   */
  function mergeConfig(base, mine, theirs) {
    const out = clone(mine);
    if (!theirs || !Array.isArray(theirs.targets)) return out;
    const baseTargets = new Map(((base && base.targets) || []).map((t) => [t.id, t]));
    for (const [index, theirTarget] of theirs.targets.entries()) {
      const baseTarget = baseTargets.get(theirTarget.id);
      const myTarget = out.targets.find((t) => t.id === theirTarget.id);
      if (!baseTarget) {
        // 밖에서 새로 추가한 폴더
        if (!myTarget) out.targets.splice(Math.min(index, out.targets.length), 0, clone(theirTarget));
        continue;
      }
      if (!myTarget) continue; // 화면에서 뺀 폴더
      const known = new Set(baseTarget.rules.map((r) => r.id));
      const have = new Set(myTarget.rules.map((r) => r.id));
      const added = theirTarget.rules.filter((r) => !known.has(r.id) && !have.has(r.id));
      if (!added.length) continue;
      // 밖에서 맨 위에 넣은 것은 맨 위에, 나머지는 맨 아래에 (규칙은 위에서부터 적용된다)
      const firstKnown = theirTarget.rules.findIndex((r) => known.has(r.id));
      const top = [];
      const bottom = [];
      for (const r of added) {
        const at = theirTarget.rules.indexOf(r);
        (firstKnown === -1 || at < firstKnown ? top : bottom).push(clone(r));
      }
      myTarget.rules = [...top, ...myTarget.rules, ...bottom];
    }
    return out;
  }

  const isAbsolute = (p) => p.startsWith('~') || p.startsWith('/') || p.startsWith('\\') || /^[A-Za-z]:/.test(p);

  /** `.`·`..` 를 글자로만 정리한 절대 경로 (Rust 의 config::resolve 와 같은 규칙) */
  function resolvePath(basePath, p, home, sep) {
    const expand = (v) => String(v).replace(/^~(?=[\\/]|$)/, home);
    const full = isAbsolute(p) ? expand(p) : `${expand(basePath)}/${p}`;
    const drive = /^[A-Za-z]:/.test(full) ? full.slice(0, 2) : '';
    const parts = [];
    for (const seg of full.slice(drive.length).split(/[\\/]+/)) {
      if (!seg || seg === '.') continue;
      if (seg === '..') parts.pop();
      else parts.push(seg);
    }
    return `${drive}${sep}${parts.join(sep)}`;
  }

  /** 규칙 목적지가 정리 대상 폴더 안인지. 밖이면 실행 단계에서 거부되므로 미리 알려 준다. */
  function destInsideTargets(dest, ownerPath, targets, home, sep) {
    if (!String(dest || '').trim()) return true;
    const full = resolvePath(ownerPath, String(dest).trim(), home, sep);
    return targets.some((t) => {
      if (!t.path) return false;
      const r = resolvePath(home, t.path, home, sep);
      return full === r || full.startsWith(r + sep);
    });
  }

  /**
   * 타격음 합성 값. 자산 파일 없이 Web Audio 로 만든다:
   * 짧은 소음(손바닥이 닿는 "찰")을 대역 필터로 걸러 빠르게 줄이고, 낮은 사인파 한 번("싹"의 몸통)을 겹친다.
   * 세기가 셀수록 크고, 낮고, 길다. 시스템 알림음을 넘지 않게 최대 음량을 0.5 아래로 둔다.
   */
  function slapSpec(level) {
    const table = {
      tap: { gain: 0.16, noiseMs: 45, bandHz: 3600, q: 1.4, thumpHz: [320, 180], thumpMs: 40, thumpGain: 0.1, echoes: 0 },
      slap: { gain: 0.3, noiseMs: 110, bandHz: 2400, q: 0.9, thumpHz: [210, 80], thumpMs: 90, thumpGain: 0.32, echoes: 0 },
      wham: { gain: 0.42, noiseMs: 170, bandHz: 1700, q: 0.7, thumpHz: [160, 48], thumpMs: 170, thumpGain: 0.46, echoes: 1 },
    };
    return table[level] || table.slap;
  }

  /** 맞은 행이 위에서부터 빠지는 순서. 행이 많으면 뒤쪽은 한꺼번에 빠진다(전체가 0.9초를 넘지 않게). */
  function staggerDelays(count, stepMs = 30, maxMs = 900) {
    const out = [];
    for (let i = 0; i < count; i += 1) out.push(Math.min(i * stepMs, maxMs));
    return out;
  }

  /** 기록 한 줄의 상태: 화면의 표시와 되돌리기 버튼을 정한다. */
  function runStatus(run) {
    const ops = run.ops || [];
    const left = run.left ?? ops.filter((o) => !o.undone && (o.type === 'move' || o.to)).length;
    if (run.state === 'running') return { key: 'running', canUndo: false, left };
    if (run.undone) return { key: 'undone', canUndo: false, left: 0 };
    const interrupted = run.state === 'interrupted';
    if (!ops.length) return { key: interrupted ? 'interrupted' : 'failed', canUndo: false, left: 0 };
    if (run.tried && left > 0) return { key: 'partial', canUndo: true, left };
    return { key: interrupted ? 'interrupted' : 'done', canUndo: left > 0 || ops.some((o) => !o.undone), left };
  }

  /** 실패한 결과만 뽑아 "파일 — 사유" 목록으로 */
  function failures(results) {
    return (results || []).filter((r) => !r.ok).map((r) => ({ name: r.name, reason: r.message }));
  }

  function fmtBytes(n) {
    if (n >= 1e9) return `${(n / 1e9).toFixed(1)}GB`;
    if (n >= 1e6) return `${Math.round(n / 1e6)}MB`;
    return `${Math.max(1, Math.round(n / 1e3))}KB`;
  }

  return { mergeConfig, resolvePath, destInsideTargets, slapSpec, staggerDelays, runStatus, failures, fmtBytes };
});
