import takesJson from './takes.json';
import syncJson from './sync.json';

export const FPS = 60;
export const W = 1920;
export const H = 1080;
export const SW = 2560; // recorded screen size
export const SH = 1440;

export type TakeName = 'launcher' | 'jev' | 'snap' | 'ocr' | 'rec' | 'clip';
export type TakeData = {
  dur: number;
  cursor: [number, number, number][];
  down: number[];
  up: number[];
  keys: [number, string][];
  chars: number[];
  marks: Record<string, number>;
};
export const TAKES = takesJson as unknown as Record<TakeName, TakeData>;
export const SYNC = syncJson as {kicks: number[]; hats: number[]; mids: number[]; downbeats: number[]};
export const DB = SYNC.downbeats; // downbeats in video seconds (drop = DB[0])

export const clamp = (v: number, a = 0, b = 1) => Math.min(b, Math.max(a, v));
export const lerp = (a: number, b: number, t: number) => a + (b - a) * t;
export const easeInOutCubic = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2);
export const easeOutCubic = (t: number) => 1 - Math.pow(1 - t, 3);
export const easeOutQuint = (t: number) => 1 - Math.pow(1 - t, 5);
export const easeInCubic = (t: number) => t * t * t;
export const easeOutExpo = (t: number) => (t >= 1 ? 1 : 1 - Math.pow(2, -10 * t));
export const easeInExpo = (t: number) => (t <= 0 ? 0 : Math.pow(2, 10 * t - 10));
export const easeOutBack = (t: number, s = 1.70158) => 1 + (s + 1) * Math.pow(t - 1, 3) + s * Math.pow(t - 1, 2);
/** smooth 0..1 progress of time `t` through window [a, a+d] */
export const prog = (t: number, a: number, d: number) => clamp((t - a) / d);

/** Damped spring response for a step at t=0 (value 0 -> 1), like a physical UI spring. */
export const springStep = (t: number, stiffness = 170, damping = 18) => {
  if (t <= 0) return 0;
  const w0 = Math.sqrt(stiffness);
  const zeta = damping / (2 * w0);
  if (zeta < 1) {
    const wd = w0 * Math.sqrt(1 - zeta * zeta);
    return 1 - Math.exp(-zeta * w0 * t) * (Math.cos(wd * t) + (zeta * w0 / wd) * Math.sin(wd * t));
  }
  return 1 - Math.exp(-w0 * t) * (1 + w0 * t);
};

/** Envelope that spikes at each kick and decays: beat-reactive punch. */
export const kickPulse = (t: number, decay = 9, from = 0, to = 999) => {
  let v = 0;
  for (const k of SYNC.kicks) {
    if (k < from || k > to) continue;
    if (k <= t && t - k < 0.6) v = Math.max(v, Math.exp(-(t - k) * decay));
  }
  return v;
};

// ---------- take segments (speed ramps + jump cuts) ----------
export type Seg = {v0: number; v1: number; t0: number; rate: number};
export const segTakeTime = (segs: Seg[], v: number): {t: number; seg: Seg; i: number} => {
  let i = segs.findIndex((s) => v >= s.v0 && v < s.v1);
  if (i < 0) i = v < segs[0].v0 ? 0 : segs.length - 1;
  const s = segs[i];
  const vv = clamp(v, s.v0, s.v1);
  return {t: s.t0 + (vv - s.v0) * s.rate, seg: s, i};
};
/** map take-time -> video time (first segment that contains it), or null */
export const takeToVideo = (segs: Seg[], t: number): number | null => {
  for (const s of segs) {
    const te = s.t0 + (s.v1 - s.v0) * s.rate;
    if (t >= s.t0 && t < te) return s.v0 + (t - s.t0) / s.rate;
  }
  return null;
};

// ---------- camera ----------
export type Cam = {t: number; x: number; y: number; z: number; d?: number};
/** Screen-Studio style camera: each key eases from the previous state to its target over d seconds (take time). */
export const cameraAt = (keys: Cam[], t: number) => {
  let x = keys[0].x, y = keys[0].y, z = keys[0].z;
  for (let i = 1; i < keys.length; i++) {
    const k = keys[i];
    if (t < k.t) break;
    const d = k.d ?? 0.7;
    const p = easeInOutCubic(clamp((t - k.t) / d));
    // zoom interpolated in log space so it feels linear to the eye
    x = lerp(x, k.x, p);
    y = lerp(y, k.y, p);
    z = Math.exp(lerp(Math.log(z), Math.log(k.z), p));
  }
  return {x, y, z};
};

// ---------- cursor ----------
export const cursorAt = (take: TakeData, t: number) => {
  const c = take.cursor;
  if (t <= c[0][0]) return {x: c[0][1], y: c[0][2], lastMove: -99};
  let lo = 0, hi = c.length - 1;
  while (lo < hi) {
    const m = (lo + hi + 1) >> 1;
    if (c[m][0] <= t) lo = m; else hi = m - 1;
  }
  const a = c[lo], b = c[Math.min(lo + 1, c.length - 1)];
  const span = b[0] - a[0];
  const p = span > 0 && span < 0.25 ? clamp((t - a[0]) / span) : 0;
  return {x: lerp(a[1], b[1], p), y: lerp(a[2], b[2], p), lastMove: a[0]};
};

export const BRAND = {
  ember: '#F2542D',
  ember2: '#FF9A5C',
  glow: '#FF7A45',
  ink: '#0B0B0E',
  grad: 'linear-gradient(90deg,#F2542D 0%,#FF7A45 45%,#FFB37A 100%)',
};
export const DISPLAY = '"Inter Display", "Inter", sans-serif';
export const UI = '"Ubuntu Sans", "Inter", sans-serif';
export const MONO = '"Ubuntu Sans Mono", "DejaVu Sans Mono", monospace';
