import React from 'react';
import {AbsoluteFill, Composition, Img, staticFile} from 'remotion';
import {Backdrop, Glyph} from './Graphics';
import {BRAND, Cam, DISPLAY, H, Seg, TakeName, UI, W} from './lib';
import {TakeView} from './Take';

// Clips and stills for the README: the real takes, Screen Studio framing.
type ClipDef = {
  id: string;
  name: TakeName;
  // [takeStart, takeEnd, playbackRate] pieces, played back to back
  pieces: [number, number, number][];
  cams: Cam[];
  cursorKinds?: [number, number, 'left_ptr' | 'hand2' | 'xterm' | 'crosshair'][];
  hideCursor?: [number, number][];
  keycaps?: boolean;
};

const toSegs = (pieces: ClipDef['pieces']): Seg[] => {
  let v = 0;
  return pieces.map(([t0, t1, rate]) => {
    const d = (t1 - t0) / rate;
    const s = {v0: v, v1: v + d, t0, rate};
    v += d;
    return s;
  });
};
const length = (pieces: ClipDef['pieces']) => pieces.reduce((a, [t0, t1, r]) => a + (t1 - t0) / r, 0);

const LAUNCHER = (z = 1.75, y = 470): Cam[] => [{t: 0, x: 1398, y, z}];

export const CLIPS: ClipDef[] = [
  {id: 'launcher', name: 'launcher', pieces: [[2.25, 11.9, 1.15]], cams: LAUNCHER(1.7), hideCursor: [[0, 99]]},
  {id: 'emoji', name: 'emoji', pieces: [[1.2, 6.9, 1]], cams: LAUNCHER(1.75, 480), hideCursor: [[0, 99]]},
  {
    id: 'jev', name: 'jev', pieces: [[1.0, 8.3, 1.15], [15.45, 18.6, 1]],
    cams: [{t: 0, x: 1398, y: 450, z: 1.7}, {t: 15.44, x: 1640, y: 860, z: 1.25, d: 0.01}, {t: 15.5, x: 1640, y: 860, z: 1.0, d: 1.0}],
    hideCursor: [[0, 99]],
  },
  {id: 'jev2', name: 'jev2', pieces: [[1.0, 24.3, 1.6]], cams: LAUNCHER(1.75, 450), hideCursor: [[0, 99]]},
  {
    id: 'snap', name: 'snap', pieces: [[1.9, 2.6, 1], [4.4, 9.2, 1.3], [10.3, 12.8, 1.2], [14.2, 16.6, 1.2], [18.7, 20.7, 1.2], [20.9, 22.5, 1], [22.5, 27.0, 1.1]],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 4.6, x: 1560, y: 580, z: 1.45, d: 0.8}, {t: 6.4, x: 1800, y: 745, z: 1.42, d: 1.4},
      {t: 10.4, x: 1850, y: 690, z: 1.8, d: 0.7}, {t: 18.85, x: 1830, y: 760, z: 1.7, d: 0.6}, {t: 20.95, x: 1500, y: 720, z: 1.08, d: 0.5},
      {t: 22.35, x: 420, y: 1180, z: 1.8, d: 0.9}],
    cursorKinds: [[2.35, 20.7, 'crosshair']],
  },
  {
    id: 'pin', name: 'pin', pieces: [[1.0, 2.0, 1], [4.9, 18.3, 1.25]],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 5.0, x: 1300, y: 420, z: 1.35, d: 0.6}, {t: 12.7, x: 1260, y: 900, z: 1.15, d: 0.6}],
    cursorKinds: [[1.3, 12.4, 'crosshair']],
  },
  {id: 'palette', name: 'palette', pieces: [[3.3, 17.6, 1.25]], cams: [{t: 0, x: 1400, y: 760, z: 1.1}, {t: 9.9, x: 1328, y: 741, z: 1.45, d: 0.6}], cursorKinds: [[3.6, 9.7, 'crosshair']]},
  {id: 'compare', name: 'compare', pieces: [[3.6, 20.2, 1.25]], cams: [{t: 0, x: 1328, y: 760, z: 1.05}, {t: 5.8, x: 1328, y: 800, z: 1.2, d: 0.6}], cursorKinds: [[7.1, 14.1, 'hand2']]},
  {
    id: 'ocr', name: 'ocr', pieces: [[1.1, 2.6, 1], [5.6, 14.4, 1.2]],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 5.9, x: 1300, y: 590, z: 1.35, d: 0.7}, {t: 9.3, x: 1320, y: 720, z: 1.3, d: 0.6}],
    cursorKinds: [[2.4, 9.3, 'crosshair']], hideCursor: [[9.4, 99]],
  },
  {id: 'table', name: 'table', pieces: [[1.0, 12.9, 1.2]], cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 4.7, x: 1700, y: 700, z: 1.4, d: 0.6}, {t: 6.4, x: 1328, y: 700, z: 1.4, d: 0.6}], cursorKinds: [[1.3, 6.5, 'crosshair']]},
  {id: 'smart', name: 'smart', pieces: [[1.0, 12.5, 1.2]], cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 6.0, x: 1000, y: 520, z: 1.3, d: 0.6}, {t: 9.1, x: 1328, y: 760, z: 1.35, d: 0.6}], cursorKinds: [[1.3, 9.1, 'crosshair']], hideCursor: [[9.2, 99]]},
  {
    id: 'rec', name: 'rec', pieces: [[1.0, 18.6, 1.3]],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 1.3, x: 1400, y: 600, z: 1.45, d: 0.6}, {t: 4.35, x: 1280, y: 330, z: 1.4, d: 0.6}, {t: 6.9, x: 1280, y: 720, z: 1.0, d: 0.9}],
    cursorKinds: [[3.2, 4.4, 'hand2']],
  },
  {id: 'clip', name: 'clip', pieces: [[0.9, 9.3, 1]], cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 1.1, x: 1400, y: 600, z: 1.38, d: 0.5}], hideCursor: [[0, 99]]},
];

const Framed: React.FC<{clip: ClipDef}> = ({clip}) => {
  const segs = toSegs(clip.pieces);
  const s = 0.9;
  return (
    <AbsoluteFill>
      <Backdrop dark={0.55} />
      <div style={{
        position: 'absolute', left: W * (1 - s) / 2, top: H * (1 - s) / 2, width: W, height: H,
        transform: `scale(${s})`, transformOrigin: '0 0', borderRadius: 26, overflow: 'hidden',
        boxShadow: '0 40px 90px rgba(0,0,0,.55), 0 0 0 1.5px rgba(255,255,255,.14)',
      }}>
        <TakeView name={clip.name} segs={segs} cams={clip.cams} cursorKinds={clip.cursorKinds} hideCursor={clip.hideCursor} keycaps={clip.keycaps ?? true} punch={false} />
      </div>
    </AbsoluteFill>
  );
};

// ---------- banner / social preview (1280x640) ----------
const Banner: React.FC = () => (
  <AbsoluteFill style={{background: '#08080b'}}>
    <div style={{position: 'absolute', inset: 0, transform: 'scale(1.0)'}}><Backdrop dark={0.62} /></div>
    <AbsoluteFill style={{background: 'radial-gradient(circle at 26% 50%, rgba(242,84,45,.28), transparent 42%)'}} />
    <div style={{position: 'absolute', left: 690, top: 120, width: 780, perspective: 1800}}>
      <Img src={staticFile('banner_shot.png')} style={{
        width: 780, borderRadius: 18, transform: 'rotateY(-16deg) rotateX(6deg)', transformOrigin: '0% 50%',
        boxShadow: '0 40px 90px rgba(0,0,0,.65), 0 0 0 1.5px rgba(255,255,255,.14)',
      }} />
    </div>
    <AbsoluteFill style={{background: 'linear-gradient(90deg, rgba(8,8,11,.55) 0%, rgba(8,8,11,.25) 45%, transparent 60%)'}} />
    <div style={{position: 'absolute', left: 92, top: 0, bottom: 0, display: 'flex', flexDirection: 'column', justifyContent: 'center', width: 560}}>
      <div style={{display: 'flex', alignItems: 'center', gap: 22}}>
        <Glyph draw={1} lens={1} dot={1} size={92} glow={0.6} />
        <div style={{fontFamily: DISPLAY, fontWeight: 800, fontSize: 84, letterSpacing: '-0.05em', color: '#fff'}}>RustCast</div>
      </div>
      <div style={{fontFamily: DISPLAY, fontWeight: 700, fontSize: 42, letterSpacing: '-0.03em', color: '#fff', marginTop: 24, lineHeight: 1.1}}>
        The launcher <span style={{backgroundImage: BRAND.grad, WebkitBackgroundClip: 'text', color: 'transparent'}}>Linux</span> deserves.
      </div>
      <div style={{fontFamily: UI, fontSize: 23, color: 'rgba(255,255,255,.72)', marginTop: 18, lineHeight: 1.45}}>
        Apps, files, maths and plain-English commands. Screenshots, OCR, screen recording and clipboard history. One keystroke away.
      </div>
      <div style={{display: 'flex', gap: 10, marginTop: 30}}>
        {['Rust', 'X11 + Wayland', 'MIT'].map((t) => (
          <div key={t} style={{fontFamily: UI, fontSize: 21, color: '#fff', padding: '8px 16px', borderRadius: 999, background: 'rgba(255,255,255,.08)', border: '1px solid rgba(255,255,255,.16)'}}>{t}</div>
        ))}
      </div>
    </div>
  </AbsoluteFill>
);

export const ReadmeCompositions: React.FC = () => (
  <>
    {CLIPS.map((c) => (
      <Composition key={c.id} id={`readme-${c.id}`} component={Framed} defaultProps={{clip: c}}
        durationInFrames={Math.round(length(c.pieces) * 60)} fps={60} width={W} height={H} />
    ))}
    <Composition id="banner" component={Banner} durationInFrames={1} fps={30} width={1280} height={640} />
  </>
);
