// Small synthesized sounds so no audio assets are needed.
let ctx: AudioContext | null = null;
let volume = 0.7;

export function setVolume(v: number) {
  volume = Math.max(0, Math.min(1, v / 100));
}

type Tone = [freq: number, dur: number, type?: OscillatorType, delay?: number, gain?: number];

const SOUNDS: Record<string, Tone[]> = {
  turn: [
    [660, 0.12, 'sine', 0],
    [880, 0.18, 'sine', 0.12],
  ],
  request: [
    [520, 0.1, 'triangle', 0],
    [520, 0.1, 'triangle', 0.16],
  ],
  ping: [[1040, 0.12, 'sine', 0, 0.6]],
  chat: [[740, 0.07, 'sine', 0, 0.4]],
  draw: [[400, 0.05, 'triangle', 0, 0.35]],
  tap: [[260, 0.04, 'square', 0, 0.15]],
  shuffle: [
    [300, 0.03, 'sawtooth', 0, 0.15],
    [340, 0.03, 'sawtooth', 0.05, 0.15],
    [320, 0.03, 'sawtooth', 0.1, 0.15],
    [360, 0.03, 'sawtooth', 0.15, 0.15],
  ],
  dice: [
    [500, 0.04, 'square', 0, 0.2],
    [700, 0.04, 'square', 0.07, 0.2],
    [600, 0.05, 'square', 0.14, 0.2],
  ],
};

export function play(name: keyof typeof SOUNDS | string) {
  const tones = SOUNDS[name];
  if (!tones || volume <= 0) return;
  try {
    ctx = ctx || new AudioContext();
    const t0 = ctx.currentTime;
    for (const [f, d, type = 'sine', delay = 0, g = 1] of tones) {
      const o = ctx.createOscillator();
      const gain = ctx.createGain();
      o.type = type;
      o.frequency.value = f;
      gain.gain.setValueAtTime(0.0001, t0 + delay);
      gain.gain.exponentialRampToValueAtTime(0.25 * volume * g, t0 + delay + 0.01);
      gain.gain.exponentialRampToValueAtTime(0.0001, t0 + delay + d);
      o.connect(gain).connect(ctx.destination);
      o.start(t0 + delay);
      o.stop(t0 + delay + d + 0.02);
    }
  } catch {
    /* audio not available */
  }
}

let flashTimer: number | null = null;
export function flashTitle(text: string) {
  if (document.hasFocus() || flashTimer) return;
  const orig = document.title;
  let on = false;
  flashTimer = window.setInterval(() => {
    document.title = (on = !on) ? text : orig;
  }, 900);
  const stop = () => {
    if (flashTimer) clearInterval(flashTimer);
    flashTimer = null;
    document.title = orig;
    window.removeEventListener('focus', stop);
  };
  window.addEventListener('focus', stop);
}
