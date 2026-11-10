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

  /** 주어진 오디오 컨텍스트(실시간이든 오프라인이든)의 `at` 시각에 타격음 하나를 놓는다. */
  function schedule(ac, level, at = ac.currentTime) {
    const spec = SmashLogic.slapSpec(level);
    const master = ac.createGain();
    master.gain.value = spec.gain;
    master.connect(ac.destination);

    const hit = (when, scale) => {
      // 손바닥이 닿는 소리: 흰 소음을 대역 필터로 거르고 아주 빨리 줄인다
      const frames = Math.max(1, Math.round((ac.sampleRate * spec.noiseMs) / 1000));
      const buffer = ac.createBuffer(1, frames, ac.sampleRate);
      const data = buffer.getChannelData(0);
      for (let i = 0; i < frames; i += 1) data[i] = Math.random() * 2 - 1;
      const noise = ac.createBufferSource();
      noise.buffer = buffer;
      const band = ac.createBiquadFilter();
      band.type = 'bandpass';
      band.frequency.value = spec.bandHz;
      band.Q.value = spec.q;
      const env = ac.createGain();
      env.gain.setValueAtTime(scale, when);
      env.gain.exponentialRampToValueAtTime(0.001, when + spec.noiseMs / 1000);
      noise.connect(band).connect(env).connect(master);
      noise.start(when);
      noise.stop(when + spec.noiseMs / 1000);

      // 몸통: 낮은 사인파가 짧게 떨어진다
      const osc = ac.createOscillator();
      osc.type = 'sine';
      osc.frequency.setValueAtTime(spec.thumpHz[0], when);
      osc.frequency.exponentialRampToValueAtTime(spec.thumpHz[1], when + spec.thumpMs / 1000);
      const body = ac.createGain();
      body.gain.setValueAtTime(spec.thumpGain * scale, when);
      body.gain.exponentialRampToValueAtTime(0.001, when + spec.thumpMs / 1000);
      osc.connect(body).connect(master);
      osc.start(when);
      osc.stop(when + spec.thumpMs / 1000);
    };

    hit(at, 1);
    // 철썩: 방에 한 번 울린다
    for (let i = 1; i <= spec.echoes; i += 1) hit(at + 0.085 * i, 0.35 / i);
    return (Math.max(spec.noiseMs, spec.thumpMs) + 85 * spec.echoes) / 1000;
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
