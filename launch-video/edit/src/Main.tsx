import React from 'react';
import {AbsoluteFill, Audio, Sequence, staticFile, useCurrentFrame} from 'remotion';
import {Backdrop, Caption, ColdOpen, EndCard, Flash, Grain, LogoSlam, Montage, RecSplit, Title, ToolStrip, Vignette} from './Graphics';
import {DB, FPS, Seg, TAKES, TakeName, clamp, easeInCubic, easeInOutCubic, easeOutExpo, prog, takeToVideo} from './lib';
import {TakeProps, TakeView} from './Take';

export const TOTAL = 69.0;

// ------------------------------------------------------------------ the cut (video seconds)
// DB = downbeats of the music cut. DB[0] = 7.242 is the drop; bars are ~1.81 s.
const S = {
  drop: DB[0], // 7.242  logo
  launch: DB[2], // 10.841 launcher · math · emoji
  jev: DB[5], // 16.228 Jev
  snap: DB[9], // 23.496 screenshot studio
  power: DB[14], // 32.505 pin · palette · compare
  ocr: DB[18], // 39.680 OCR code · table · smart actions
  rec: DB[22], // 46.878 recorder
  clip: DB[26], // 54.169 clipboard
  montage: DB[28], // 57.768 beat montage
  end: DB[30], // 61.367 end card
};
const SNAP = S.snap - 21.684; // the screenshot take was cut for an earlier slot; shift it
const REC = S.rec - 36.104;

const seg = (v0: number, v1: number, t0: number, t1: number): Seg => ({v0, v1, t0, rate: (t1 - t0) / (v1 - v0)});
const dimTitle = (t0: number, d = 0.95) => (v: number) => (v < t0 + d ? 1 : 1 - easeInOutCubic(clamp((v - (t0 + d)) / 0.35)));
const LAUNCHER_CAM = (z = 1.75, y = 470) => [{t: 0, x: 1398, y, z}];

type Shot = TakeProps & {from: number; to: number; cut?: 'whip' | 'soft' | 'none'};
const SHOTS: Shot[] = [
  // ---- launcher
  {
    name: 'launcher', from: 10.3, to: S.launch + 3.622, cut: 'none',
    segs: [seg(10.3, 11.6, 1.336, 2.636), seg(11.6, 12.9, 2.636, 4.6), seg(12.9, S.launch + 3.622, 5.35, 8.95)],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 2.4, x: 1398, y: 470, z: 1.7, d: 0.6}, {t: 5.4, x: 1398, y: 440, z: 1.95, d: 0.6}, {t: 8.62, x: 1398, y: 470, z: 1.7, d: 0.4}],
    hideCursor: [[0, 99]],
    float: (v) => 1 - easeInOutCubic(prog(v, 10.45, 0.9)),
  },
  {
    name: 'emoji', from: S.launch + 3.622, to: S.jev, cut: 'soft',
    segs: [seg(S.launch + 3.622, S.jev, 3.9, 6.5)],
    cams: LAUNCHER_CAM(1.85, 480), hideCursor: [[0, 99]],
  },
  // ---- Jev
  {
    name: 'jev2', from: S.jev, to: DB[7], cut: 'whip',
    segs: [seg(S.jev, 17.25, 0.25, 1.27), seg(17.25, 18.1, 5.0, 7.1), seg(18.1, 18.98, 10.9, 12.75), seg(18.98, DB[7], 15.3, 17.1)],
    cams: [{t: 0, x: 1398, y: 470, z: 1.6}, {t: 1.2, x: 1398, y: 450, z: 1.85, d: 0.5}],
    hideCursor: [[0, 99]], dim: dimTitle(S.jev),
  },
  {
    name: 'jev', from: DB[7], to: S.snap, cut: 'soft',
    segs: [seg(DB[7], 21.3, 3.0, 6.3), seg(21.3, 22.2, 6.3, 7.95), seg(22.2, S.snap, 15.45, 16.85)],
    cams: [{t: 0, x: 1398, y: 450, z: 1.85}, {t: 15.44, x: 1700, y: 860, z: 1.3, d: 0.01}, {t: 15.5, x: 1640, y: 860, z: 1.0, d: 1.0}],
    hideCursor: [[0, 99]],
  },
  // ---- screenshot studio
  {
    name: 'snap', from: S.snap, to: S.power, cut: 'whip',
    segs: [seg(21.684, 22.85, 1.24, 2.406), seg(22.85, 24.35, 4.6, 8.4), seg(24.35, 25.25, 10.4, 12.7), seg(25.25, 26.15, 14.3, 16.45),
      seg(26.15, 27.05, 18.85, 20.6), seg(27.05, 28.45, 20.95, 22.75), seg(28.45, 30.694, 22.75, 26.25)].map((s) => ({...s, v0: s.v0 + SNAP, v1: s.v1 + SNAP})),
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 4.6, x: 1560, y: 580, z: 1.45, d: 0.8}, {t: 6.4, x: 1800, y: 745, z: 1.42, d: 1.4},
      {t: 10.4, x: 1850, y: 690, z: 1.85, d: 0.7}, {t: 18.85, x: 1830, y: 760, z: 1.75, d: 0.6}, {t: 20.95, x: 1500, y: 720, z: 1.08, d: 0.5},
      {t: 22.35, x: 420, y: 1180, z: 1.85, d: 0.9}],
    cursorKinds: [[2.35, 20.7, 'crosshair']], dim: dimTitle(S.snap),
  },
  // ---- power tools: pin · palette · compare
  {
    name: 'pin', from: S.power, to: 34.9, cut: 'whip',
    segs: [seg(S.power, 33.3, 2.0, 2.8), seg(33.3, 33.95, 10.9, 12.35), seg(33.95, 34.35, 12.7, 13.5), seg(34.35, 34.9, 15.85, 17.4)],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 10.6, x: 700, y: 430, z: 1.85, d: 0.35}, {t: 12.6, x: 1250, y: 1020, z: 1.22, d: 0.45}, {t: 15.7, x: 1220, y: 900, z: 1.18, d: 0.6}],
    cursorKinds: [[1.3, 12.4, 'crosshair']], dim: dimTitle(S.power, 0.8),
  },
  {
    name: 'palette', from: 34.9, to: 37.2, cut: 'soft',
    segs: [seg(34.9, 35.75, 7.5, 9.7), seg(35.75, 37.2, 10.6, 15.0)],
    cams: [{t: 0, x: 2050, y: 900, z: 1.3}, {t: 10.0, x: 1328, y: 741, z: 1.6, d: 0.5}],
    cursorKinds: [[3.6, 9.7, 'crosshair']],
  },
  {
    name: 'compare', from: 37.2, to: S.ocr, cut: 'soft',
    segs: [seg(37.2, 38.55, 6.6, 11.0), seg(38.55, S.ocr, 17.85, 19.9)],
    cams: [{t: 0, x: 1328, y: 830, z: 1.25}, {t: 17.8, x: 1328, y: 820, z: 1.22, d: 0.4}],
    cursorKinds: [[7.1, 14.1, 'hand2']],
  },
  // ---- OCR: code · table · smart actions
  {
    name: 'ocr', from: S.ocr, to: 43.0, cut: 'whip',
    segs: [seg(S.ocr, 40.75, 1.29, 2.36), seg(40.75, 41.85, 5.9, 9.3), seg(41.85, 43.0, 10.0, 12.3)],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 5.9, x: 1300, y: 590, z: 1.4, d: 0.6}, {t: 9.3, x: 1320, y: 720, z: 1.35, d: 0.5}],
    cursorKinds: [[2.4, 9.3, 'crosshair']], hideCursor: [[9.4, 99]], dim: dimTitle(S.ocr),
  },
  {
    name: 'table', from: 43.0, to: 44.9, cut: 'soft',
    segs: [seg(43.0, 44.9, 8.0, 11.8)],
    cams: [{t: 0, x: 1328, y: 700, z: 1.5}],
  },
  {
    name: 'smart', from: 44.9, to: S.rec, cut: 'soft',
    segs: [seg(44.9, S.rec, 9.4, 12.4)],
    cams: [{t: 0, x: 1328, y: 760, z: 1.42}], hideCursor: [[0, 99]],
  },
  // ---- recorder
  {
    name: 'rec', from: S.rec, to: 40.35 + REC, cut: 'whip',
    segs: [seg(36.104, 37.25, 0.253, 1.4), seg(37.25, 38.45, 2.6, 4.9), seg(38.45, 40.35, 6.3, 12.7)].map((s) => ({...s, v0: s.v0 + REC, v1: s.v1 + REC})),
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 1.3, x: 1400, y: 600, z: 1.5, d: 0.6}, {t: 4.35, x: 1280, y: 330, z: 1.5, d: 0.6}, {t: 6.9, x: 1280, y: 720, z: 1.0, d: 0.9}],
    cursorKinds: [[3.2, 4.4, 'hand2']], dim: dimTitle(S.rec),
  },
  // ---- clipboard
  {
    name: 'clip', from: S.clip, to: S.montage, cut: 'whip',
    segs: [seg(S.clip, 54.95, 0.45, 1.231), seg(54.95, S.montage, 1.231, 6.0)],
    cams: [{t: 0, x: 1280, y: 720, z: 1}, {t: 1.1, x: 1400, y: 600, z: 1.38, d: 0.5}],
    hideCursor: [[0, 99]], dim: dimTitle(S.clip, 0.75),
  },
];

/** Shot wrapper: zoom-blur whip on section changes, a softer punch on in-section cuts. */
const Shell: React.FC<{from: number; to: number; children: React.ReactNode; cut?: 'whip' | 'soft' | 'none'}> = ({from, to, children, cut = 'whip'}) => {
  const v = useCurrentFrame() / FPS;
  if (v < from || v >= to) return null;
  const amt = cut === 'whip' ? 1 : cut === 'soft' ? 0.45 : 0;
  const i = amt * (1 - easeOutExpo(clamp((v - from) / (cut === 'soft' ? 0.22 : 0.32))));
  const o = cut === 'whip' ? easeInCubic(clamp((v - (to - 0.12)) / 0.12)) * 0.6 : 0;
  return (
    <AbsoluteFill style={{
      transform: `scale(${1 + i * 0.12 + o * 0.08})`,
      filter: i + o > 0.01 ? `blur(${(i + o) * 16}px)` : undefined,
    }}>{children}</AbsoluteFill>
  );
};

// ------------------------------------------------------------------ sound design
type Sfx = [number, string, number];
const mapTake = (name: TakeName, list: number[], file: string, vol: number): Sfx[] =>
  SHOTS.filter((s) => s.name === name).flatMap((shot) =>
    list.map((t) => takeToVideo(shot.segs, t)).filter((v): v is number => v !== null && v >= shot.from && v < shot.to).map((v): Sfx => [v, file, vol]));
const MONTAGE_AT = Array.from({length: 8}, (_, i) => S.montage + i * ((S.end - S.montage) / 8));
const SFX: Sfx[] = [
  ...[0.63, 1.8, 2.43, 3.6, 5.38].map((t): Sfx => [t, 'pop', 0.22]),
  [5.62, 'riser', 0.42], [6.06, 'click', 0.6], [6.64, 'click', 0.6],
  [7.16, 'whoosh', 0.45], [S.drop, 'boom', 0.75], [10.3, 'whoosh', 0.4],
  ...[S.jev, S.snap, S.power, S.ocr, S.rec, S.clip].map((t): Sfx => [t - 0.04, 'swish', 0.38]),
  ...SHOTS.filter((s) => s.cut === 'soft').map((s): Sfx => [s.from - 0.03, 'swish', 0.2]),
  [40.33 + REC, 'whoosh', 0.42],
  ...mapTake('launcher', TAKES.launcher.chars, 'key', 0.11),
  ...mapTake('emoji', TAKES.emoji.chars, 'key', 0.1),
  ...mapTake('jev2', TAKES.jev2.chars, 'key', 0.07),
  ...mapTake('jev', TAKES.jev.chars, 'key', 0.07),
  ...mapTake('snap', TAKES.snap.down, 'click', 0.28),
  ...mapTake('pin', TAKES.pin.down, 'click', 0.28),
  ...mapTake('palette', TAKES.palette.down, 'click', 0.28),
  ...mapTake('compare', TAKES.compare.down, 'click', 0.28),
  ...mapTake('table', TAKES.table.down, 'click', 0.28),
  ...mapTake('rec', TAKES.rec.down, 'click', 0.3),
  ...mapTake('snap', [22.18], 'shutter', 0.45),
  ...mapTake('pin', [12.76], 'pop', 0.35),
  ...mapTake('ocr', [9.263], 'shutter', 0.4),
  ...MONTAGE_AT.map((t): Sfx => [t - 0.03, 'swish', 0.24]),
  [S.end, 'boom', 0.42],
];

export const Main: React.FC = () => {
  const v = useCurrentFrame() / FPS;
  return (
    <AbsoluteFill style={{background: '#000'}}>
      <Audio src={staticFile('music.wav')} />
      {SFX.filter(([, , vol]) => vol > 0).map(([t, f, vol], i) => (
        <Sequence key={i} from={Math.max(0, Math.round(t * FPS))} durationInFrames={Math.round(2 * FPS)} layout="none">
          <Audio src={staticFile(`sfx/${f}.wav`)} volume={vol} />
        </Sequence>
      ))}

      {/* 0 – cold open over the breakdown */}
      <Shell from={0} to={S.drop + 0.6} cut="none"><ColdOpen /></Shell>

      {/* screen takes */}
      {SHOTS.map((sh, i) => (
        <Shell key={i} from={sh.from} to={sh.to} cut={sh.cut}>
          {sh.name === 'launcher' && <Backdrop dark={0.7} />}
          <TakeView {...sh} />
        </Shell>
      ))}

      {/* 1 – drop: logo slam */}
      <Shell from={S.drop} to={11.0} cut="none"><LogoSlam t0={S.drop} out={10.3} /></Shell>

      {/* feature strips: one visual system for every section */}
      <ToolStrip start={11.55} end={S.jev - 0.15} items={[['Apps', 11.6], ['Math', 12.9], ['Emoji', S.launch + 3.622]]} />
      <Title t0={S.jev} kicker="Meet Jev" line="Just say it." accent={['say']} />
      <ToolStrip start={17.2} end={S.snap - 0.15} items={[['Create', 17.25], ['Screenshot', 18.1], ['Record', 18.98], ['Multi-step', DB[7]], ['Done', 22.25]]} />
      <Title t0={S.snap} kicker="Screenshot studio" line="Capture. Mark up. Ship it." accent={['Mark', 'up.']} />
      <ToolStrip start={22.75 + SNAP} end={S.power - 0.15} items={[['Select', 22.85 + SNAP], ['Arrow', 24.35 + SNAP], ['Steps', 25.25 + SNAP], ['Spotlight', 26.15 + SNAP], ['Beautify', 27.13 + SNAP], ['Drag & drop', 28.6 + SNAP]]} />
      <Title t0={S.power} dur={0.8} kicker="Power tools" line="Pin it. Pick it. Compare it." accent={['Pick', 'Compare']} />
      <ToolStrip start={33.25} end={S.ocr - 0.15} items={[['Censor', 33.3], ['Pin on top', 33.95], ['Colour palette', 34.9], ['Compare', 37.2], ['Differences', 38.55]]} />
      <Title t0={S.ocr} kicker="OCR" line="Copy text from anything." accent={['anything.']} />
      <ToolStrip start={40.7} end={S.rec - 0.15} items={[['Text', 40.75], ['Code', 41.85], ['Tables', 43.0], ['Links · QR', 44.9]]} />
      <Title t0={S.rec} kicker="Screen recorder" line="Lock onto one window." accent={['one']} />
      <Caption t0={38.5 + REC} t1={40.2 + REC} text="Drag anything over it" icon="•" pos="bottom" />
      <RecSplit t0={40.35 + REC} t1={S.clip} screenFrom={12.7} outFrom={6.2} />
      <Title t0={S.clip} dur={0.75} kicker="Clipboard history" line="Never lose a copy." accent={['Never']} />
      <Caption t0={55.1} t1={57.55} text="Text & images · search · persisted" icon="•" pos="bottom" />

      {/* montage, one feature per beat */}
      <Montage end={S.end} items={[
        {word: 'JEV', take: 'jev2', at: MONTAGE_AT[0], from: 16.5},
        {word: 'CAPTURE', take: 'snap', at: MONTAGE_AT[1], from: 19.6},
        {word: 'PIN', take: 'pin', at: MONTAGE_AT[2], from: 16.8},
        {word: 'COLOURS', take: 'palette', at: MONTAGE_AT[3], from: 13.5},
        {word: 'COMPARE', take: 'compare', at: MONTAGE_AT[4], from: 18.6},
        {word: 'OCR', take: 'table', at: MONTAGE_AT[5], from: 11.3},
        {word: 'RECORD', take: 'rec', at: MONTAGE_AT[6], from: 10.0},
        {word: 'CLIPBOARD', take: 'clip', at: MONTAGE_AT[7], from: 5.4},
      ]} />

      <EndCard t0={S.end} total={TOTAL} />

      {/* global finishing */}
      <Flash at={22.2} dur={0.16} peak={0.25} />
      <Vignette strength={0.5} />
      <Grain opacity={0.06} />
      <AbsoluteFill style={{background: '#000', opacity: 1 - clamp(v / 0.25) + clamp((v - (TOTAL - 0.3)) / 0.3), pointerEvents: 'none'}} />
    </AbsoluteFill>
  );
};
