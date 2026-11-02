'use strict';
// 자동 정리: 켜 둔 폴더에 새 파일이 들어오면, 다 받아질 때까지 기다렸다가 규칙대로 정리한다.
// AI 제안은 쓰지 않고 사용자가 만든 규칙만 적용한다.
const fs = require('fs');
const path = require('path');
const { expandHome } = require('./config');
const { scanTarget, applyPlan } = require('./organizer');

const EVENT_DEBOUNCE_MS = 1500;
const SWEEP_INTERVAL_MS = 10 * 60 * 1000;
const RETRY_WATCH_MS = 30 * 1000;

class AutoTidy {
  /**
   * @param {{ load: () => object, onRun?: (target, result) => void, onError?: (target, err) => void }} opts
   */
  constructor({ load, onRun = () => {}, onError = () => {} }) {
    this.load = load;
    this.onRun = onRun;
    this.onError = onError;
    this.watchers = new Map(); // targetId → { root, watcher }
    this.timers = new Map(); // targetId → timeout
    this.sizes = new Map(); // 파일 경로 → 지난번 확인한 크기
    this.sweep = null;
  }

  // 설정이 바뀔 때마다 부른다. 자동 정리가 켜진 폴더만 감시한다.
  sync(cfg = this.load()) {
    const wanted = new Map(
      cfg.targets.filter((t) => t.auto).map((t) => [t.id, path.resolve(expandHome(t.path))]),
    );
    for (const [id, entry] of this.watchers) {
      if (wanted.get(id) !== entry.root) this.unwatch(id);
    }
    for (const [id, root] of wanted) {
      if (!this.watchers.has(id)) this.watch(id, root);
    }
    if (wanted.size && !this.sweep) {
      this.sweep = setInterval(() => this.watchers.forEach((_e, id) => this.schedule(id, 0)), SWEEP_INTERVAL_MS);
    } else if (!wanted.size && this.sweep) {
      clearInterval(this.sweep);
      this.sweep = null;
    }
    return [...this.watchers.keys()];
  }

  watch(id, root) {
    try {
      const watcher = fs.watch(root, { persistent: true }, () => this.schedule(id, EVENT_DEBOUNCE_MS));
      watcher.on('error', (err) => {
        this.unwatch(id);
        this.onError({ id, root }, err);
        setTimeout(() => this.sync(), RETRY_WATCH_MS).unref?.();
      });
      this.watchers.set(id, { root, watcher });
      // 켜자마자 지금 쌓여 있는 파일도 한 번 본다.
      this.schedule(id, EVENT_DEBOUNCE_MS);
    } catch (err) {
      this.onError({ id, root }, err);
    }
  }

  unwatch(id) {
    const entry = this.watchers.get(id);
    if (entry) entry.watcher.close();
    this.watchers.delete(id);
    clearTimeout(this.timers.get(id));
    this.timers.delete(id);
    const prefix = entry ? entry.root + path.sep : null;
    if (prefix) for (const file of this.sizes.keys()) if (file.startsWith(prefix)) this.sizes.delete(file);
  }

  schedule(id, ms) {
    clearTimeout(this.timers.get(id));
    const timer = setTimeout(() => this.check(id), ms);
    timer.unref?.();
    this.timers.set(id, timer);
  }

  // 수정 후 settle 시간이 지났고, 지난 확인 때와 크기가 같은 파일만 '다 받아진 파일'로 본다.
  check(id) {
    this.timers.delete(id);
    const entry = this.watchers.get(id);
    if (!entry) return null;
    let cfg;
    let scan;
    try {
      cfg = this.load();
      const target = cfg.targets.find((t) => t.id === id);
      if (!target?.auto) return null;
      scan = scanTarget(cfg, id);
    } catch (err) {
      this.onError({ id, root: entry.root }, err);
      return null;
    }
    const settleMs = cfg.settings.autoDelaySec * 1000;
    const now = Date.now();
    const ready = [];
    const seen = new Set();
    let waiting = false;
    for (const item of scan.items) {
      let st;
      try {
        st = fs.statSync(item.src);
      } catch {
        continue;
      }
      seen.add(item.src);
      const prev = this.sizes.get(item.src);
      this.sizes.set(item.src, st.size);
      if (now - st.mtimeMs >= settleMs && prev === st.size) ready.push(item);
      else waiting = true;
    }
    // 사라진 파일 기록은 지운다(맵이 계속 커지지 않게).
    for (const file of this.sizes.keys()) if (file.startsWith(entry.root + path.sep) && !seen.has(file)) this.sizes.delete(file);
    if (waiting) this.schedule(id, settleMs);
    if (!ready.length) return null;
    const result = applyPlan(cfg, ready, 'auto');
    for (const item of ready) this.sizes.delete(item.src);
    this.onRun({ id, root: entry.root }, result);
    return result;
  }

  stop() {
    for (const id of [...this.watchers.keys()]) this.unwatch(id);
    if (this.sweep) clearInterval(this.sweep);
    this.sweep = null;
  }
}

module.exports = { AutoTidy };
