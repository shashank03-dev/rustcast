import React from 'react';
import {AbsoluteFill, Img, OffthreadVideo, Sequence, staticFile, useCurrentFrame} from 'remotion';
import cursors from '../public/cursor/cursors.json';
import {
  BRAND, Cam, FPS, H, SH, SW, Seg, TAKES, TakeName, UI, W, cameraAt, clamp, cursorAt, easeOutBack, easeOutCubic,
  kickPulse, lerp, prog, segTakeTime, takeToVideo,
} from './lib';

type CursorKind = 'left_ptr' | 'hand2' | 'xterm' | 'crosshair';

export type TakeProps = {
  name: TakeName;
  segs: Seg[];
  cams: Cam[];
  /** take-time ranges with a specific cursor shape */
  cursorKinds?: [number, number, CursorKind][];
  /** take-time ranges where the pointer is hidden */
  hideCursor?: [number, number][];
  /** 0 = full bleed, 1 = floating card in 3D space */
  float?: (v: number) => number;
  /** extra blur/dim on the take (title overlays) */
  dim?: (v: number) => number;
  keycaps?: boolean;
  punch?: boolean;
  src?: string;
};

const CURSOR_PX = 46; // drawn size in recorded-screen pixels

export const TakeView: React.FC<TakeProps> = ({
  name, segs, cams, cursorKinds = [], hideCursor = [], float, dim, keycaps = true, punch = true, src,
}) => {
  const frame = useCurrentFrame();
  const v = frame / FPS;
  const take = TAKES[name];
  const {t, seg} = segTakeTime(segs, v);
  const cam = cameraAt(cams, t);
  const f = float ? clamp(float(v)) : 0;
  const dm = dim ? clamp(dim(v)) : 0;

  // ---- camera transform (screen px -> composition px) ----
  const base = W / SW; // 0.75
  const pk = punch ? kickPulse(v, 10) * 0.012 : 0;
  const s = base * cam.z * (1 + pk);
  // keep the screen edges out of frame while full-bleed
  const halfW = W / 2 / s, halfH = H / 2 / s;
  const cx = cam.z >= 1 ? clamp(cam.x, halfW, SW - halfW) : SW / 2;
  const cy = cam.z >= 1 ? clamp(cam.y, halfH, SH - halfH) : SH / 2;
  const tx = W / 2 - cx * s;
  const ty = H / 2 - cy * s;

  // ---- floating card (3D) ----
  const cardScale = lerp(1, 0.8, f);
  const rx = lerp(0, 16, f);
  const ry = lerp(0, -10, f);
  const radius = lerp(0, 22, f);

  // ---- cursor ----
  const cp = cursorAt(take, t);
  let kind: CursorKind = 'left_ptr';
  for (const [a, b, k] of cursorKinds) if (t >= a && t < b) kind = k;
  const meta = (cursors as Record<string, {w: number; h: number; hx: number; hy: number}>)[kind];
  const cs = CURSOR_PX / meta.w;
  let cOpacity = 1;
  for (const [a, b] of hideCursor) {
    if (t >= a - 0.15 && t < b + 0.15) cOpacity = Math.min(cOpacity, 1 - Math.min(clamp((t - (a - 0.15)) / 0.15), clamp(((b + 0.15) - t) / 0.15)));
  }
  // pressed state
  const lastDown = take.down.filter((d) => d <= t).pop() ?? -99;
  const lastUp = take.up.filter((u) => u <= t).pop() ?? -99;
  const pressed = lastDown > lastUp;
  const pressScale = pressed ? 0.86 : lerp(0.86, 1, easeOutBack(clamp((t - lastUp) / 0.18)));

  const ripples = take.down
    .map((d) => ({d, age: (t - d) / seg.rate}))
    .filter((r) => r.age >= 0 && r.age < 0.55);

  return (
    <AbsoluteFill style={{perspective: 2600, perspectiveOrigin: '50% 40%'}}>
      <AbsoluteFill
        style={{
          transform: `scale(${cardScale}) rotateX(${rx}deg) rotateY(${ry}deg)`,
          transformOrigin: '50% 55%',
          borderRadius: radius,
          overflow: 'hidden',
          boxShadow: f > 0.01 ? `0 ${60 * f}px ${140 * f}px rgba(0,0,0,${0.65 * f}), 0 0 0 ${1.5 * f}px rgba(255,255,255,${0.12 * f})` : 'none',
        }}
      >
        <div
          style={{
            position: 'absolute', left: 0, top: 0, width: SW, height: SH,
            transform: `translate(${tx}px, ${ty}px) scale(${s})`, transformOrigin: '0 0',
            filter: dm > 0.001 ? `blur(${dm * 18}px) brightness(${1 - dm * 0.55})` : undefined,
          }}
        >
          {segs.map((sg, i) => (
            <Sequence key={i} from={Math.round(sg.v0 * FPS)} durationInFrames={Math.max(1, Math.round((sg.v1 - sg.v0) * FPS))} layout="none">
              <OffthreadVideo
                src={staticFile(src ?? `takes/${name}.mp4`)}
                trimBefore={Math.round(sg.t0 * FPS)}
                playbackRate={sg.rate}
                muted
                style={{position: 'absolute', left: 0, top: 0, width: SW, height: SH}}
              />
            </Sequence>
          ))}
          {/* click ripples */}
          {ripples.map((r) => {
            const p = clamp(r.age / 0.55);
            return (
              <div key={r.d} style={{
                position: 'absolute', left: cp.x, top: cp.y, width: 0, height: 0,
              }}>
                <div style={{
                  position: 'absolute', left: -lerp(12, 54, easeOutCubic(p)), top: -lerp(12, 54, easeOutCubic(p)),
                  width: lerp(24, 108, easeOutCubic(p)), height: lerp(24, 108, easeOutCubic(p)), borderRadius: '50%',
                  border: `${lerp(5, 1.5, p)}px solid ${BRAND.glow}`, opacity: (1 - p) * 0.9,
                  boxShadow: `0 0 ${24 * (1 - p)}px ${BRAND.glow}`,
                }} />
              </div>
            );
          })}
          {/* the pointer */}
          <Img
            src={staticFile(`cursor/${kind}.png`)}
            style={{
              position: 'absolute', left: cp.x - meta.hx * cs, top: cp.y - meta.hy * cs,
              width: meta.w * cs, height: meta.h * cs, opacity: cOpacity,
              transform: `scale(${pressScale})`, transformOrigin: `${meta.hx * cs}px ${meta.hy * cs}px`,
              filter: 'drop-shadow(0 3px 5px rgba(0,0,0,.45))',
            }}
          />
        </div>
      </AbsoluteFill>
      {keycaps && <Keycaps name={name} segs={segs} v={v} />}
    </AbsoluteFill>
  );
};

// ---------- keystroke overlay ----------
const KEYMAP: Record<string, string[]> = {
  'Alt Space': ['Alt', 'Space'],
  'Super Shift S': ['Super', 'Shift', 'S'],
  'Super Shift T': ['Super', 'Shift', 'T'],
  'Super Shift R': ['Super', 'Shift', 'R'],
  'Super Shift C': ['Super', 'Shift', 'C'],
  'Ctrl B': ['Ctrl', 'B'],
};
export const Keycap: React.FC<{label: string; big?: boolean; glow?: number}> = ({label, big, glow = 0}) => (
  <div style={{
    minWidth: big ? 150 : 64, height: big ? 128 : 64, padding: big ? '0 34px' : '0 18px', borderRadius: big ? 26 : 14,
    display: 'flex', alignItems: 'center', justifyContent: 'center',
    background: 'linear-gradient(180deg,#3a3a40 0%,#232327 100%)',
    boxShadow: `inset 0 1.5px 0 rgba(255,255,255,.22), inset 0 -${big ? 7 : 4}px 0 rgba(0,0,0,.45), 0 ${big ? 18 : 8}px ${big ? 40 : 18}px rgba(0,0,0,.5), 0 0 ${60 * glow}px rgba(255,122,69,${0.8 * glow})`,
    border: `1px solid rgba(255,255,255,${0.1 + 0.4 * glow})`,
    color: '#f4f4f6', fontFamily: UI, fontWeight: 600, fontSize: big ? 52 : 28, letterSpacing: '-0.01em',
  }}>{label}</div>
);

const Keycaps: React.FC<{name: TakeName; segs: Seg[]; v: number}> = ({name, segs, v}) => {
  const take = TAKES[name];
  const evs = take.keys
    .map(([t, label]) => ({vt: takeToVideo(segs, t), label}))
    .filter((e): e is {vt: number; label: string} => e.vt !== null && e.vt <= v && v - e.vt < 0.9);
  const e = evs.pop();
  if (!e) return null;
  const age = v - e.vt;
  const keys = KEYMAP[e.label] ?? [e.label];
  const inP = easeOutBack(clamp(age / 0.22), 2.2);
  const out = clamp((age - 0.68) / 0.2);
  return (
    <div style={{
      position: 'absolute', left: 0, right: 0, bottom: 64, display: 'flex', justifyContent: 'center',
      opacity: 1 - out, transform: `translateY(${(1 - inP) * 30 + out * 10}px) scale(${lerp(0.85, 1, inP)})`,
    }}>
      <div style={{
        display: 'flex', gap: 12, padding: 12, borderRadius: 22, alignItems: 'center',
        background: 'rgba(14,14,18,.62)', backdropFilter: 'blur(18px)', border: '1px solid rgba(255,255,255,.08)',
      }}>
        {keys.map((k, i) => {
          const kp = clamp((age - i * 0.05) / 0.12);
          return (
            <div key={i} style={{transform: `translateY(${(1 - kp) * 6}px)`}}>
              <Keycap label={k} glow={Math.exp(-age * 5)} />
            </div>
          );
        })}
      </div>
    </div>
  );
};
