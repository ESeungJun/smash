'use strict';
/* global SmashLogic */
// 타격음. 소리 파일 없이 Web Audio 로 그때그때 만든다. 값은 logic.js 의 slapSpec 이 정한다.
(() => {
  let ctx = null;

  function context() {
    const Ctor = window.AudioContext || window.webkitAudioContext;
    if (!Ctor) return null;
    if (!ctx) ctx = new Ctor();
    if (ctx.state === 'suspended') ctx.resume().catch(() => {});
    return ctx;
  }

  /** 흰 소음 버퍼 (ms 길이) */
  function noiseBuffer(ac, ms) {
    const frames = Math.max(1, Math.round((ac.sampleRate * ms) / 1000));
    const buffer = ac.createBuffer(1, frames, ac.sampleRate);
    const data = buffer.getChannelData(0);
    for (let i = 0; i < frames; i += 1) data[i] = Math.random() * 2 - 1;
    return buffer;
  }

  /** 주어진 오디오 컨텍스트(실시간이든 오프라인이든)의 `at` 시각에 타격음 하나를 놓는다. 소리 길이(초)를 돌려준다. */
  function schedule(ac, level, at = ac.currentTime) {
    const spec = SmashLogic.slapSpec(level);
    const master = ac.createGain();
    master.gain.value = spec.gain;
    master.connect(ac.destination);

    // 소음 길: 고역 통과 → (대역 필터로 살린 울림 + 그대로 둔 날카로운 앞소리)
    const high = ac.createBiquadFilter();
    high.type = 'highpass';
    high.frequency.value = spec.highHz;
    const band = ac.createBiquadFilter();
    band.type = 'bandpass';
    band.frequency.value = spec.bandHz;
    band.Q.value = spec.q;
    const bandGain = ac.createGain();
    bandGain.gain.value = 1.6;
    const crack = ac.createGain();
    crack.gain.value = 0.55;
    high.connect(band).connect(bandGain).connect(master);
    high.connect(crack).connect(master);

    const burst = (when, ms, peak) => {
      const src = ac.createBufferSource();
      src.buffer = noiseBuffer(ac, ms);
      const env = ac.createGain();
      env.gain.setValueAtTime(peak, when);
      env.gain.exponentialRampToValueAtTime(0.001, when + ms / 1000);
      src.connect(env).connect(high);
      src.start(when);
      src.stop(when + ms / 1000);
    };

    const hit = (when, scale) => {
      // 짝: 손가락·손바닥이 차례로 닿는 아주 짧은 터짐 몇 번, 마지막이 가장 세다
      for (let b = 0; b < spec.bursts; b += 1) {
        burst(when + (b * spec.burstGapMs) / 1000, spec.burstMs, scale * (b === spec.bursts - 1 ? 1 : 0.6));
      }
      // 꼬리: 마지막 터짐에서 짧게 줄어든다
      const tailAt = when + ((spec.bursts - 1) * spec.burstGapMs) / 1000;
      burst(tailAt, spec.tailMs, scale * 0.9);

      // 몸통: 낮은 사인파가 짧게 떨어진다
      const osc = ac.createOscillator();
      osc.type = 'sine';
      osc.frequency.setValueAtTime(spec.thumpHz[0], tailAt);
      osc.frequency.exponentialRampToValueAtTime(spec.thumpHz[1], tailAt + spec.thumpMs / 1000);
      const body = ac.createGain();
      body.gain.setValueAtTime(spec.thumpGain * scale, tailAt);
      body.gain.exponentialRampToValueAtTime(0.001, tailAt + spec.thumpMs / 1000);
      osc.connect(body).connect(master);
      osc.start(tailAt);
      osc.stop(tailAt + spec.thumpMs / 1000);
    };

    hit(at, 1);
    // 철썩: 방에 한 번 울린다
    for (let i = 1; i <= spec.echoes; i += 1) hit(at + 0.085 * i, 0.35 / i);
    const lead = (spec.bursts - 1) * spec.burstGapMs;
    return (lead + Math.max(spec.tailMs, spec.thumpMs) + 85 * spec.echoes) / 1000;
  }

  window.SmashSound = {
    /** 지금 바로 한 번 친다. 소리를 낼 수 없는 환경이면 조용히 넘어간다. */
    play(level) {
      try {
        const ac = context();
        if (ac) schedule(ac, level);
      } catch {
        // 소리는 덤이다. 실패해도 정리는 그대로 간다
      }
    },
    /** 첫 클릭 때 미리 깨워 둔다(브라우저는 사용자 동작 뒤에만 소리를 허용한다) */
    warm() {
      try {
        context();
      } catch {
        // 위와 같다
      }
    },
    schedule,
  };
})();
