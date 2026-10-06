import React from 'react';
import {AbsoluteFill, Audio, Sequence, staticFile, useCurrentFrame} from 'remotion';
import {Backdrop, Caption, ColdOpen, EndCard, Flash, Grain, LogoSlam, Montage, RecSplit, Title, ToolStrip, Vignette} from './Graphics';
import {Cam, DB, FPS, Seg, TAKES, TakeName, clamp, easeInCubic, easeInOutCubic, easeOutExpo, lerp, prog, takeToVideo} from './lib';
import {TakeProps, TakeView} from './Take';

// ------------------------------------------------------------------ the cut (video seconds)
// downbeats: DB[0]=7.242 (drop) ... sections land on 4-bar / 3-bar phrase boundaries
const S = {
  drop: DB[0], // 7.242
  launch: DB[2], // 10.841
  jev: DB[5], // 16.228
  snap: DB[8], // 21.684
  ocr: DB[13], // 30.694
  rec: DB[16], // 36.104
  clip: DB[20], // 43.325
  montage: DB[23], // 48.712
  end: DB[25], // 52.358
};

const seg = (v0: number, v1: number, t0: number, t1: number): Seg => ({v0, v1, t0, rate: (t1 - t0) / (v1 - v0)});
const dimTitle = (t0: number, d = 0.95) => (v: number) => (v < t0 + d ? 1 : 1 - easeInOutCubic(clamp((v - (t0 + d)) / 0.35)));

type Shot = TakeProps & {from: number; to: number};
const SHOTS: Shot[] = [
  {
    name: 'launcher', from: 10.3, to: S.jev,
    segs: [seg(10.3, 11.6, 1.336, 2.636), seg(11.6, 13.34, 2.636, 4.9), seg(13.34, S.jev, 5.35, 8.95)],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 2.4, x: 1398, y: 470, z: 1.7, d: 0.6}, {t: 5.4, x: 1398, y: 440, z: 1.95, d: 0.7}, {t: 8.62, x: 1398, y: 470, z: 1.62, d: 0.5}],
    hideCursor: [[0, 99]],
    float: (v) => 1 - easeInOutCubic(prog(v, 10.45, 0.9)),
  },
  {
    name: 'jev', from: S.jev, to: S.snap,
    segs: [seg(S.jev, 17.25, 0.32, 1.343), seg(17.25, 19.45, 1.343, 6.2), seg(19.45, 20.4, 6.2, 7.95), seg(20.4, S.snap, 15.45, 16.85)],
    cams: [{t: 0, x: 1398, y: 470, z: 1.62}, {t: 1.25, x: 1398, y: 440, z: 1.8, d: 0.6}, {t: 15.44, x: 1700, y: 860, z: 1.3, d: 0.01}, {t: 15.5, x: 1640, y: 860, z: 1.0, d: 1.1}],
    hideCursor: [[0, 99]],
    dim: dimTitle(S.jev),
  },
  {
    name: 'snap', from: S.snap, to: S.ocr,
    segs: [seg(S.snap, 22.85, 1.24, 2.406), seg(22.85, 24.35, 4.6, 8.4), seg(24.35, 25.25, 10.4, 12.7), seg(25.25, 26.15, 14.3, 16.45),
      seg(26.15, 27.05, 18.85, 20.6), seg(27.05, 28.45, 20.95, 22.75), seg(28.45, S.ocr, 22.75, 26.25)],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 4.6, x: 1560, y: 580, z: 1.45, d: 0.8}, {t: 6.4, x: 1800, y: 745, z: 1.42, d: 1.4},
      {t: 10.4, x: 1850, y: 690, z: 1.85, d: 0.7}, {t: 18.85, x: 1830, y: 760, z: 1.75, d: 0.6}, {t: 20.95, x: 1500, y: 720, z: 1.08, d: 0.5},
      {t: 22.35, x: 420, y: 1180, z: 1.85, d: 0.9}],
    cursorKinds: [[2.35, 20.7, 'crosshair']],
    dim: dimTitle(S.snap),
  },
  {
    name: 'ocr', from: S.ocr, to: S.rec,
    segs: [seg(S.ocr, 31.8, 1.293, 2.4), seg(31.8, 33.3, 5.9, 9.3), seg(33.3, S.rec, 9.3, 12.9)],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 5.9, x: 1300, y: 590, z: 1.4, d: 0.9}, {t: 9.3, x: 1320, y: 720, z: 1.28, d: 0.7}],
    cursorKinds: [[2.4, 9.3, 'crosshair']],
    hideCursor: [[9.4, 99]],
    dim: dimTitle(S.ocr),
  },
  {
    name: 'rec', from: S.rec, to: 40.35,
    segs: [seg(S.rec, 37.25, 0.253, 1.4), seg(37.25, 38.45, 2.6, 4.9), seg(38.45, 40.35, 6.3, 12.7)],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 1.3, x: 1400, y: 600, z: 1.5, d: 0.6}, {t: 4.35, x: 1280, y: 330, z: 1.5, d: 0.6}, {t: 6.9, x: 1280, y: 720, z: 1.0, d: 0.9}],
    cursorKinds: [[3.2, 4.4, 'hand2']],
    dim: dimTitle(S.rec),
  },
  {
    name: 'clip', from: S.clip, to: S.montage,
    segs: [seg(S.clip, 44.4, 0.226, 1.301), seg(44.4, S.montage, 1.301, 8.45)],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 1.2, x: 1400, y: 600, z: 1.38, d: 0.6}],
    hideCursor: [[0, 99]],
    dim: dimTitle(S.clip),
  },
];

/** Section wrapper: zoom-blur whip in, quick blur out, on downbeats. */
const Shell: React.FC<{from: number; to: number; children: React.ReactNode; whip?: boolean}> = ({from, to, children, whip = true}) => {
  const v = useCurrentFrame() / FPS;
  if (v < from || v >= to) return null;
  const i = whip ? 1 - easeOutExpo(clamp((v - from) / 0.32)) : 0;
  const o = whip ? easeInCubic(clamp((v - (to - 0.12)) / 0.12)) : 0;
  return (
    <AbsoluteFill style={{
      transform: `scale(${1 + i * 0.12 + o * 0.08})`,
      filter: i + o > 0.01 ? `blur(${(i + o) * 16}px)` : undefined,
    }}>{children}</AbsoluteFill>
  );
};

// ------------------------------------------------------------------ sound design
type Sfx = [number, string, number];
const mapTake = (name: TakeName, list: number[], file: string, vol: number): Sfx[] => {
  const shot = SHOTS.find((s) => s.name === name)!;
  return list.map((t) => takeToVideo(shot.segs, t)).filter((v): v is number => v !== null && v >= shot.from && v < shot.to).map((v) => [v, file, vol]);
};
const SFX: Sfx[] = [
  ...[0.63, 1.8, 2.43, 3.6, 5.38].map((t): Sfx => [t, 'pop', 0.22]),
  [5.62, 'riser', 0.42], [6.06, 'click', 0.6], [6.64, 'click', 0.6],
  [7.16, 'whoosh', 0.45], [S.drop, 'boom', 0.75],
  [10.3, 'whoosh', 0.4],
  ...[S.jev, S.snap, S.ocr, S.rec, S.clip].map((t): Sfx => [t - 0.04, 'swish', 0.38]),
  [20.38, 'swish', 0.3], [40.33, 'whoosh', 0.42],
  ...mapTake('launcher', TAKES.launcher.chars, 'key', 0.11),
  ...mapTake('jev', TAKES.jev.chars, 'key', 0.09),
  ...mapTake('snap', TAKES.snap.down, 'click', 0.28),
  ...mapTake('rec', TAKES.rec.down, 'click', 0.3),
  ...mapTake('snap', [22.18], 'shutter', 0.45),
  ...mapTake('ocr', [9.263], 'shutter', 0.4),
  ...[48.712, 49.168, 49.847, 50.521, 51.2, 51.879].map((t): Sfx => [t - 0.03, 'swish', 0.26]),
  [S.end, 'boom', 0.42],
];

export const Main: React.FC = () => {
  const v = useCurrentFrame() / FPS;
  return (
    <AbsoluteFill style={{background: '#000'}}>
      <Audio src={staticFile('music.wav')} />
      {SFX.map(([t, f, vol], i) => (
        <Sequence key={i} from={Math.max(0, Math.round(t * FPS))} durationInFrames={Math.round(2 * FPS)} layout="none">
          <Audio src={staticFile(`sfx/${f}.wav`)} volume={vol} />
        </Sequence>
      ))}

      {/* 0 – cold open over the breakdown */}
      <Shell from={0} to={S.drop + 0.6} whip={false}><ColdOpen /></Shell>

      {/* screen takes */}
      {SHOTS.map((sh) => (
        <Shell key={sh.name} from={sh.from} to={sh.to} whip={sh.name !== 'launcher'}>
          {sh.name === 'launcher' && <Backdrop dark={0.7} />}
          <TakeView {...sh} />
        </Shell>
      ))}

      {/* 1 – drop: logo slam (over the beginning of the hero fly-in) */}
      <Shell from={S.drop} to={11.0} whip={false}><LogoSlam t0={S.drop} out={10.3} /></Shell>

      {/* section titles + captions */}
      <Caption t0={11.75} t1={13.2} text="Launch anything" icon="•" />
      <Caption t0={13.75} t1={16.0} text="Instant math, copied with ↵" icon="•" />
      <Title t0={S.jev} kicker="Meet Jev" line="Just say it." accent={['say']} />
      <Caption t0={17.4} t1={19.4} text="Plain words → real actions" icon="•" />
      <Caption t0={20.55} t1={21.55} text="Done. Folder open." icon="•" pos="bottom" />
      <Title t0={S.snap} kicker="Screenshot studio" line="Capture. Mark up. Ship it." accent={['Mark', 'up.']} />
      <ToolStrip start={22.75} end={30.55} items={[['Select', 22.85], ['Arrow', 24.35], ['Steps', 25.25], ['Spotlight', 26.15], ['Beautify', 27.13], ['Drag & drop', 28.6]]} />
      <Title t0={S.ocr} kicker="OCR" line="Copy text from anything." accent={['anything.']} />
      <Caption t0={33.6} t1={35.95} text="Text · Code · Table — indentation kept" icon="•" />
      <Title t0={S.rec} kicker="Screen recorder" line="Lock onto one window." accent={['one']} />
      <Caption t0={38.5} t1={40.2} text="Drag anything over it" icon="•" pos="bottom" />
      <RecSplit t0={40.35} t1={S.clip} screenFrom={12.7} outFrom={6.2} />
      <Title t0={S.clip} kicker="Clipboard history" line="Never lose a copy." accent={['Never']} />
      <Caption t0={44.6} t1={48.5} text="Text & images · search · persisted" icon="•" pos="bottom" />

      {/* montage on the kicks */}
      <Montage end={S.end} items={[
        {word: 'LAUNCH', take: 'launcher', at: 48.712, from: 4.0},
        {word: 'JEV', take: 'jev', at: 49.168, from: 5.9},
        {word: 'CAPTURE', take: 'snap', at: 49.847, from: 19.6},
        {word: 'OCR', take: 'ocr', at: 50.521, from: 10.9},
        {word: 'RECORD', take: 'rec', at: 51.2, from: 10.0},
        {word: 'CLIPBOARD', take: 'clip', at: 51.879, from: 5.4},
      ]} />

      <EndCard t0={S.end} />

      {/* global finishing */}
      <Flash at={20.4} dur={0.16} peak={0.25} />
      <Vignette strength={0.5} />
      <Grain opacity={0.06} />
      {/* letterbox fade in / out */}
      <AbsoluteFill style={{background: '#000', opacity: 1 - clamp(v / 0.25) + clamp((v - 59.7) / 0.3), pointerEvents: 'none'}} />
      {void lerp}
    </AbsoluteFill>
  );
};
