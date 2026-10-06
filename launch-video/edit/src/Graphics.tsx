import React from 'react';
import {AbsoluteFill, Img, OffthreadVideo, Sequence, staticFile, useCurrentFrame} from 'remotion';
import {
  BRAND, DISPLAY, FPS, H, MONO, SH, SW, UI, W, clamp, easeInCubic, easeInExpo, easeOutBack, easeOutCubic, easeOutExpo,
  easeOutQuint, kickPulse, lerp, prog, springStep,
} from './lib';
import {Keycap} from './Take';

const useV = () => useCurrentFrame() / FPS;

// deterministic pseudo random
const rnd = (i: number) => {
  const x = Math.sin(i * 127.1 + 311.7) * 43758.5453;
  return x - Math.floor(x);
};

// ---------------------------------------------------------------- backdrop / finishing
const BD = [55, 70, 72, 74, 78, 80, 86];
export const Backdrop: React.FC<{dark?: number; zoom?: number; hue?: number}> = ({dark = 0.55, zoom = 1}) => {
  const d = BD.reduce((b, x) => (Math.abs(x - dark * 100) < Math.abs(b - dark * 100) ? x : b), BD[0]);
  return (
  <AbsoluteFill style={{background: '#07070a'}}>
    <Img src={staticFile(`bd_${d}.png`)} style={{
      position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'cover',
      transform: `scale(${1.08 * zoom})`,
    }} />
    <AbsoluteFill style={{background: 'radial-gradient(ellipse at 50% 120%, rgba(242,84,45,.16), transparent 60%)'}} />
  </AbsoluteFill>
  );
};

export const Grain: React.FC<{opacity?: number}> = ({opacity = 0.07}) => {
  const f = useCurrentFrame();
  return (
    <AbsoluteFill style={{pointerEvents: 'none', mixBlendMode: 'overlay', opacity}}>
      <svg width={W} height={H}>
        <filter id="g">
          <feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="2" seed={f % 97} stitchTiles="stitch" />
          <feColorMatrix type="saturate" values="0" />
        </filter>
        <rect width={W} height={H} filter="url(#g)" />
      </svg>
    </AbsoluteFill>
  );
};

export const Vignette: React.FC<{strength?: number}> = ({strength = 0.55}) => (
  <AbsoluteFill style={{pointerEvents: 'none', background: `radial-gradient(ellipse at 50% 50%, transparent 55%, rgba(0,0,0,${strength}) 100%)`}} />
);

export const Flash: React.FC<{at: number; dur?: number; color?: string; peak?: number}> = ({at, dur = 0.35, color = '#fff', peak = 1}) => {
  const v = useV();
  if (v < at - 0.05 || v > at + dur) return null;
  const p = v < at ? clamp((v - (at - 0.05)) / 0.05) : 1 - easeOutCubic(clamp((v - at) / dur));
  return <AbsoluteFill style={{background: color, opacity: p * peak, mixBlendMode: 'screen'}} />;
};

// ---------------------------------------------------------------- kinetic text
export const Words: React.FC<{
  text: string; t0: number; size: number; weight?: number; accent?: string[]; stagger?: number; out?: number;
  color?: string; font?: string; tracking?: string; blurIn?: boolean;
}> = ({text, t0, size, weight = 800, accent = [], stagger = 0.045, out, color = '#fff', font = DISPLAY, tracking = '-0.045em', blurIn = true}) => {
  const v = useV();
  const words = text.split(' ');
  return (
    <div style={{display: 'flex', flexWrap: 'wrap', justifyContent: 'center', gap: `0 ${size * 0.26}px`, lineHeight: 1.02}}>
      {words.map((w, i) => {
        const p = clamp((v - t0 - i * stagger) / 0.42);
        const e = easeOutExpo(p);
        const o = out !== undefined ? easeInCubic(clamp((v - out - i * 0.02) / 0.22)) : 0;
        const isAcc = accent.includes(w.replace(/[.,!?]/g, ''));
        return (
          <span key={i} style={{display: 'inline-block', overflow: 'hidden', paddingBottom: size * 0.12, marginBottom: -size * 0.12}}>
            <span style={{
              display: 'inline-block', fontFamily: font, fontWeight: weight, fontSize: size, letterSpacing: tracking,
              color: isAcc ? 'transparent' : color,
              backgroundImage: isAcc ? BRAND.grad : undefined, WebkitBackgroundClip: isAcc ? 'text' : undefined, backgroundClip: isAcc ? 'text' : undefined,
              transform: `translateY(${(1 - e) * 105 - o * 60}%) rotate(${(1 - e) * 4}deg)`,
              filter: blurIn ? `blur(${(1 - e) * 10 + o * 14}px)` : undefined,
              opacity: (p > 0 ? 1 : 0) * (1 - o),
            }}>{w}</span>
          </span>
        );
      })}
    </div>
  );
};

// ---------------------------------------------------------------- 0: cold open (breakdown)
const PHRASES: [number, string, string[]][] = [
  [0.63, 'Every app.', ['app']],
  [1.8, 'Every file.', ['file']],
  [2.43, 'Every screenshot.', ['screenshot']],
  [3.6, 'Every recording.', ['recording']],
];
export const ColdOpen: React.FC = () => {
  const v = useV();
  const pre = easeInExpo(prog(v, 6.75, 0.5)); // suck-in before the drop
  return (
    <AbsoluteFill style={{background: '#000'}}>
      <AbsoluteFill style={{opacity: lerp(0, 1, prog(v, 0, 1.6)), transform: `scale(${lerp(1.18, 1.05, v / 7.2) + pre * 0.4})`}}>
        <Backdrop dark={0.78} />
      </AbsoluteFill>
      {/* light sweep */}
      <AbsoluteFill style={{
        background: `linear-gradient(105deg, transparent ${lerp(-30, 110, v / 7.2) - 12}%, rgba(255,140,90,.10) ${lerp(-30, 110, v / 7.2)}%, transparent ${lerp(-30, 110, v / 7.2) + 12}%)`,
      }} />
      <AbsoluteFill style={{alignItems: 'center', justifyContent: 'center', transform: `scale(${1 + pre * 0.35})`, filter: `blur(${pre * 8}px)`}}>
        {PHRASES.map(([t0, txt, acc], i) => {
          const next = i < PHRASES.length - 1 ? PHRASES[i + 1][0] : 5.2;
          if (v < t0 - 0.05 || v > next) return null;
          return (
            <div key={i} style={{position: 'absolute'}}>
              <Words text={txt} t0={t0 + 0.02} size={150} accent={acc} out={next - 0.26} />
            </div>
          );
        })}
        {v > 5.3 && (
          <div style={{position: 'absolute', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 46}}>
            <Words text="One keystroke away." t0={5.38} size={128} accent={['keystroke']} />
            <div style={{display: 'flex', alignItems: 'center', gap: 26}}>
              {[['Alt', 6.06], ['Space', 6.64]].map(([k, t], i) => {
                const a = clamp((v - (t as number)) / 0.3);
                const press = Math.exp(-Math.max(0, v - (t as number)) * 9);
                return (
                  <React.Fragment key={k as string}>
                    {i === 1 && <div style={{fontFamily: DISPLAY, fontSize: 70, color: 'rgba(255,255,255,.5)', opacity: a, fontWeight: 300}}>+</div>}
                    <div style={{opacity: a > 0 ? 1 : 0, transform: `translateY(${(1 - easeOutBack(a, 2.4)) * 60 + press * 10}px) scale(${lerp(0.6, 1, easeOutBack(a, 2))})`}}>
                      <Keycap label={k as string} big glow={a > 0 ? 0.35 + press * 0.65 : 0} />
                    </div>
                  </React.Fragment>
                );
              })}
            </div>
          </div>
        )}
      </AbsoluteFill>
      <Flash at={7.242} dur={0.5} />
    </AbsoluteFill>
  );
};

// ---------------------------------------------------------------- 1: logo slam on the drop
export const Glyph: React.FC<{draw: number; lens: number; dot: number; size: number; glow?: number}> = ({draw, lens, dot, size, glow = 0}) => (
  <svg viewBox="190 80 780 780" width={size} height={size} style={{overflow: 'visible'}}>
    <defs>
      <radialGradient id="emb"><stop offset="0" stopColor="#FFE9DC" /><stop offset=".45" stopColor="#FF9A5C" /><stop offset="1" stopColor="#F2542D" /></radialGradient>
      <filter id="gl" x="-100%" y="-100%" width="300%" height="300%"><feGaussianBlur stdDeviation="30" /></filter>
    </defs>
    <path d="M262.51,439.19 A258.0,258.0 0 1 1 596.65,779.21" fill="none" stroke="#F5F5F7" strokeWidth="74" strokeLinecap="round"
      pathLength={1} strokeDasharray="1 1" strokeDashoffset={1 - draw} />
    <g transform={`translate(742 292) scale(${lens}) translate(-742 -292)`}>
      <circle cx="742" cy="292" r="148" fill="#F5F5F7" fillOpacity=".14" stroke="#F5F5F7" strokeOpacity=".5" strokeWidth="24" />
    </g>
    <g transform={`translate(596.6 779.2) scale(${dot}) translate(-596.6 -779.2)`}>
      <circle cx="596.6" cy="779.2" r={120 + glow * 120} fill="#F2542D" opacity={0.35 + glow * 0.4} filter="url(#gl)" />
      <circle cx="596.6" cy="779.2" r="70.3" fill="url(#emb)" />
    </g>
  </svg>
);

const Sparks: React.FC<{t0: number; cx: number; cy: number; n?: number; spread?: number}> = ({t0, cx, cy, n = 46, spread = 900}) => {
  const v = useV();
  const a = v - t0;
  if (a < 0 || a > 1.6) return null;
  return (
    <>
      {Array.from({length: n}).map((_, i) => {
        const ang = rnd(i) * Math.PI * 2;
        const sp = (0.35 + rnd(i + 50) * 0.65) * spread;
        const d = easeOutExpo(clamp(a / 1.2)) * sp;
        const sz = 2 + rnd(i + 9) * 5;
        const op = (1 - clamp(a / (0.7 + rnd(i + 3) * 0.9)));
        return (
          <div key={i} style={{
            position: 'absolute', left: cx + Math.cos(ang) * d, top: cy + Math.sin(ang) * d + a * a * 120,
            width: sz * 3.2, height: sz, borderRadius: sz, background: i % 3 ? BRAND.ember2 : '#fff',
            transform: `rotate(${ang}rad)`, opacity: op, boxShadow: `0 0 ${sz * 3}px ${BRAND.ember}`,
          }} />
        );
      })}
    </>
  );
};

export const LogoSlam: React.FC<{t0: number; out: number}> = ({t0, out}) => {
  const v = useV();
  const a = v - t0;
  const shake = a > 0 && a < 0.45 ? Math.exp(-a * 9) * 16 : 0;
  const sx = Math.sin(a * 90) * shake, sy = Math.cos(a * 77) * shake;
  const draw = easeOutExpo(clamp(a / 0.55));
  const dot = springStep(a - 0.08, 260, 14);
  const lens = springStep(a - 0.22, 200, 13);
  const glow = Math.exp(-Math.max(0, a) * 3) + kickPulse(v, 7, t0, out) * 0.6;
  const slide = easeInOutQuart(clamp((a - 0.3) / 0.6));
  const o = easeInCubic(clamp((v - out) / 0.45));
  const zoomIn = lerp(1.35, 1, easeOutQuint(clamp(a / 0.7)));
  const word = 'RustCast';
  const ring = clamp(a / 0.9);
  return (
    <AbsoluteFill style={{transform: `translate(${sx}px,${sy}px) scale(${zoomIn * (1 - o * 0.25)})`, opacity: 1 - o, filter: `blur(${o * 20}px)`}}>
      <Backdrop dark={0.72} zoom={1 + a * 0.02} />
      <AbsoluteFill style={{background: `radial-gradient(circle at 50% 50%, rgba(242,84,45,${0.35 * glow}), transparent 45%)`}} />
      {/* shockwave */}
      <div style={{
        position: 'absolute', left: W / 2, top: H / 2, width: 0, height: 0,
      }}>
        <div style={{
          position: 'absolute', width: lerp(40, 2600, easeOutExpo(ring)), height: lerp(40, 2600, easeOutExpo(ring)),
          left: -lerp(20, 1300, easeOutExpo(ring)), top: -lerp(20, 1300, easeOutExpo(ring)), borderRadius: '50%',
          border: `${lerp(30, 1, ring)}px solid rgba(255,154,92,${(1 - ring) * 0.7})`, boxShadow: `0 0 80px rgba(242,84,45,${(1 - ring) * 0.6})`,
        }} />
      </div>
      <Sparks t0={t0} cx={W / 2} cy={H / 2} />
      <AbsoluteFill style={{alignItems: 'center', justifyContent: 'center'}}>
        <div style={{display: 'flex', alignItems: 'center', transform: `translateY(-40px)`}}>
          <div style={{transform: `translateX(${lerp(0, -20, slide)}px)`, marginRight: lerp(-250, 40, slide)}}>
            <Glyph draw={draw} lens={lens} dot={dot} size={lerp(300, 210, slide)} glow={glow} />
          </div>
          <div style={{display: 'flex', overflow: 'hidden', paddingBottom: 20, width: lerp(0, 700, slide)}}>
            {word.split('').map((c, i) => {
              const p = easeOutExpo(clamp((a - 0.42 - i * 0.035) / 0.5));
              return (
                <span key={i} style={{
                  display: 'inline-block', fontFamily: DISPLAY, fontWeight: 800, fontSize: 168, letterSpacing: '-0.05em', color: '#fff',
                  transform: `translateY(${(1 - p) * 110}%)`, opacity: p > 0 ? 1 : 0,
                }}>{c}</span>
              );
            })}
          </div>
        </div>
        <div style={{position: 'absolute', top: H / 2 + 120}}>
          <Words text="The launcher Linux deserves." t0={t0 + 1.79} size={58} weight={600} accent={['Linux']} color="rgba(255,255,255,.82)" tracking="-0.02em" />
        </div>
      </AbsoluteFill>
    </AbsoluteFill>
  );
};
const easeInOutQuart = (t: number) => (t < 0.5 ? 8 * t * t * t * t : 1 - Math.pow(-2 * t + 2, 4) / 2);

// ---------------------------------------------------------------- section title (over a dimmed take)
export const Title: React.FC<{t0: number; dur?: number; kicker?: string; line: string; accent?: string[]; sub?: string}> = ({t0, dur = 0.95, kicker, line, accent = [], sub}) => {
  const v = useV();
  if (v < t0 - 0.05 || v > t0 + dur + 0.3) return null;
  const outAt = t0 + dur;
  const kp = easeOutExpo(clamp((v - t0) / 0.35));
  const ko = clamp((v - outAt) / 0.2);
  return (
    <AbsoluteFill style={{alignItems: 'center', justifyContent: 'center', flexDirection: 'column', gap: 22}}>
      {kicker && (
        <div style={{
          fontFamily: MONO, fontSize: 30, letterSpacing: '0.18em', color: BRAND.ember2, textTransform: 'uppercase',
          opacity: kp * (1 - ko), transform: `translateY(${(1 - kp) * 20 - ko * 20}px)`,
          padding: '8px 18px', border: '1px solid rgba(255,154,92,.4)', borderRadius: 999, background: 'rgba(242,84,45,.08)',
        }}>{kicker}</div>
      )}
      <Words text={line} t0={t0 + 0.06} size={124} accent={accent} out={outAt} />
      {sub && (
        <div style={{opacity: kp * (1 - ko), transform: `translateY(${(1 - kp) * 16}px)`}}>
          <Words text={sub} t0={t0 + 0.2} size={44} weight={500} color="rgba(255,255,255,.7)" tracking="-0.01em" out={outAt} />
        </div>
      )}
    </AbsoluteFill>
  );
};

// ---------------------------------------------------------------- caption pill (while a take plays)
export const Caption: React.FC<{t0: number; t1: number; text: string; icon?: string; pos?: 'top' | 'bottom'}> = ({t0, t1, text, icon, pos = 'top'}) => {
  const v = useV();
  if (v < t0 || v > t1 + 0.3) return null;
  const p = easeOutBack(clamp((v - t0) / 0.35), 1.6);
  const o = easeInCubic(clamp((v - t1) / 0.25));
  return (
    <div style={{
      position: 'absolute', left: 0, right: 0, [pos]: 56, display: 'flex', justifyContent: 'center',
      opacity: clamp(p) * (1 - o), transform: `translateY(${(1 - p) * (pos === 'top' ? -30 : 30)}px) scale(${lerp(0.9, 1, clamp(p))})`,
    }}>
      <div style={{
        display: 'flex', alignItems: 'center', gap: 16, padding: '16px 30px', borderRadius: 999,
        background: 'rgba(12,12,16,.66)', backdropFilter: 'blur(20px)', border: '1px solid rgba(255,255,255,.1)',
        boxShadow: '0 20px 50px rgba(0,0,0,.4)',
        fontFamily: DISPLAY, fontWeight: 700, fontSize: 40, color: '#fff', letterSpacing: '-0.02em',
      }}>
        {icon && <span style={{
          width: 14, height: 14, borderRadius: 7, background: BRAND.ember, boxShadow: `0 0 18px ${BRAND.ember}`,
        }} />}
        {text}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------- tool strip (screenshot studio)
export const ToolStrip: React.FC<{items: [string, number][]; end: number; start: number}> = ({items, start, end}) => {
  const v = useV();
  if (v < start || v > end + 0.3) return null;
  const p = easeOutExpo(clamp((v - start) / 0.5));
  const o = easeInCubic(clamp((v - end) / 0.25));
  let active = -1;
  items.forEach(([, t], i) => { if (v >= t) active = i; });
  return (
    <div style={{
      position: 'absolute', left: 0, right: 0, top: 46, display: 'flex', justifyContent: 'center',
      opacity: p * (1 - o), transform: `translateY(${(1 - p) * -40}px)`,
    }}>
      <div style={{
        display: 'flex', gap: 8, padding: 8, borderRadius: 999, background: 'rgba(12,12,16,.7)', backdropFilter: 'blur(20px)',
        border: '1px solid rgba(255,255,255,.1)', boxShadow: '0 20px 50px rgba(0,0,0,.45)',
      }}>
        {items.map(([label, t], i) => {
          const on = i === active;
          const done = i < active;
          const ap = easeOutCubic(clamp((v - t) / 0.25));
          return (
            <div key={label} style={{
              padding: '12px 24px', borderRadius: 999, fontFamily: DISPLAY, fontWeight: 650, fontSize: 30, letterSpacing: '-0.01em',
              color: on ? '#fff' : done ? 'rgba(255,255,255,.75)' : 'rgba(255,255,255,.38)',
              background: on ? `linear-gradient(90deg, rgba(242,84,45,${0.95 * ap}), rgba(255,122,69,${0.95 * ap}))` : 'transparent',
              boxShadow: on ? `0 0 ${30 * ap}px rgba(242,84,45,.6)` : 'none',
              transform: `scale(${on ? lerp(1, 1.06, Math.exp(-(v - t) * 6)) : 1})`,
            }}>{done ? '✓ ' : ''}{label}</div>
          );
        })}
      </div>
    </div>
  );
};

// ---------------------------------------------------------------- recorder split screen
export const RecSplit: React.FC<{t0: number; t1: number; screenFrom: number; outFrom: number}> = ({t0, t1, screenFrom, outFrom}) => {
  const v = useV();
  if (v < t0 || v > t1 + 0.05) return null;
  const a = v - t0;
  const p = easeOutQuint(clamp(a / 0.7));
  const o = easeInCubic(clamp((v - (t1 - 0.25)) / 0.25));
  const pw = 820, ph = pw * 9 / 16;
  const panel = (side: -1 | 1, label: string, node: React.ReactNode, accent: boolean, delay: number) => {
    const q = easeOutQuint(clamp((a - delay) / 0.75));
    return (
      <div style={{
        position: 'absolute', top: 300, left: side < 0 ? 110 : W - 110 - pw, width: pw,
        transform: `perspective(2000px) translateX(${(1 - q) * side * 500}px) rotateY(${(1 - q) * side * -35 + side * -6}deg) scale(${lerp(0.8, 1, q)})`,
        opacity: q,
      }}>
        <div style={{
          width: pw, height: ph, borderRadius: 18, overflow: 'hidden', position: 'relative',
          boxShadow: accent ? `0 30px 80px rgba(0,0,0,.6), 0 0 0 2px rgba(255,122,69,.85), 0 0 60px rgba(242,84,45,.45)` : '0 30px 80px rgba(0,0,0,.6), 0 0 0 1.5px rgba(255,255,255,.18)',
        }}>{node}</div>
        <div style={{display: 'flex', alignItems: 'center', gap: 14, marginTop: 28, justifyContent: 'center',
          fontFamily: DISPLAY, fontWeight: 700, fontSize: 40, color: '#fff', letterSpacing: '-0.02em'}}>
          <span style={{width: 16, height: 16, borderRadius: 8, background: accent ? '#ff3b30' : 'rgba(255,255,255,.5)',
            boxShadow: accent ? `0 0 ${12 + 12 * Math.sin(a * 8)}px #ff3b30` : 'none'}} />
          {label}
        </div>
      </div>
    );
  };
  return (
    <AbsoluteFill style={{opacity: 1 - o}}>
      <AbsoluteFill style={{opacity: p}}><Backdrop dark={0.8} /></AbsoluteFill>
      <div style={{position: 'absolute', top: 118, left: 0, right: 0}}>
        <Words text="Overlaps never show up." t0={t0 + 0.15} size={84} accent={['never']} />
      </div>
      {panel(-1, 'Your screen', (
        <Sequence from={Math.round(t0 * FPS)} layout="none">
          <OffthreadVideo src={staticFile('takes/rec.mp4')} trimBefore={Math.round(screenFrom * FPS)} muted style={{width: pw, height: ph}} />
        </Sequence>
      ), false, 0.05)}
      {panel(1, 'Your recording', (
        <Sequence from={Math.round(t0 * FPS)} layout="none">
          <OffthreadVideo src={staticFile('takes/rec_output.mp4')} trimBefore={Math.round(outFrom * FPS)} muted style={{width: pw, height: ph}} />
        </Sequence>
      ), true, 0.18)}
      {/* arrow between */}
      <div style={{position: 'absolute', left: W / 2 - 60, top: 300 + ph / 2 - 40, width: 120, height: 80, display: 'flex', alignItems: 'center', justifyContent: 'center',
        opacity: easeOutCubic(clamp((a - 0.6) / 0.3)), transform: `translateX(${Math.sin(a * 6) * 6}px)`}}>
        <svg width="90" height="50" viewBox="0 0 90 50"><path d="M5 25 H75 M58 8 L80 25 L58 42" stroke={BRAND.ember2} strokeWidth="7" fill="none" strokeLinecap="round" strokeLinejoin="round" /></svg>
      </div>
    </AbsoluteFill>
  );
};

// ---------------------------------------------------------------- montage (kick-synced feature wall)
export type MontageItem = {word: string; take: string; at: number; from: number};
export const Montage: React.FC<{items: MontageItem[]; end: number}> = ({items, end}) => {
  const v = useV();
  if (v < items[0].at || v > end) return null;
  let idx = 0;
  items.forEach((it, i) => { if (v >= it.at) idx = i; });
  const it = items[idx];
  const a = v - it.at;
  const nextAt = idx < items.length - 1 ? items[idx + 1].at : end;
  const p = easeOutExpo(clamp(a / 0.35));
  const cw = 1040, ch = cw * 9 / 16;
  const tilt = (idx % 2 ? 1 : -1) * lerp(7, 2.5, p);
  return (
    <AbsoluteFill style={{background: '#060608'}}>
      <Backdrop dark={0.86} />
      {/* giant outlined word, behind */}
      <AbsoluteFill style={{alignItems: 'center', justifyContent: 'center'}}>
        <div style={{
          fontFamily: DISPLAY, fontWeight: 900, fontSize: 420, letterSpacing: '-0.06em', color: 'transparent',
          WebkitTextStroke: '2px rgba(255,154,92,.28)', whiteSpace: 'nowrap',
          transform: `scale(${lerp(1.3, 1.05, p) + a * 0.05}) translateX(${(idx % 2 ? -1 : 1) * a * 90}px)`,
        }}>{it.word}</div>
      </AbsoluteFill>
      <AbsoluteFill style={{alignItems: 'center', justifyContent: 'center', perspective: 2200}}>
        <div style={{
          width: cw, height: ch, borderRadius: 20, overflow: 'hidden', position: 'relative',
          transform: `translateY(-90px) rotateY(${tilt}deg) rotateX(${lerp(10, 4, p)}deg) scale(${lerp(0.72, 0.92, p) + a * 0.03})`,
          boxShadow: '0 50px 120px rgba(0,0,0,.7), 0 0 0 1.5px rgba(255,255,255,.14)',
        }}>
          {items.map((m, i) => (
            <Sequence key={i} from={Math.round(m.at * FPS)} durationInFrames={Math.round(((i < items.length - 1 ? items[i + 1].at : end) - m.at) * FPS)} layout="none">
              <div style={{position: 'absolute', inset: 0, transform: 'scale(1.6)', transformOrigin: '50% 40%'}}>
                <OffthreadVideo src={staticFile(`takes/${m.take}.mp4`)} trimBefore={Math.round(m.from * FPS)} muted style={{width: cw, height: ch}} />
              </div>
            </Sequence>
          ))}
        </div>
      </AbsoluteFill>
      {/* solid word, in front */}
      <div style={{position: 'absolute', left: 0, right: 0, bottom: 118, display: 'flex', justifyContent: 'center'}}>
        <div style={{display: 'flex', overflow: 'hidden', paddingBottom: 16}}>
          {it.word.split('').map((c, i) => {
            const q = easeOutExpo(clamp((a - i * 0.018) / 0.28));
            return <span key={i} style={{display: 'inline-block', fontFamily: DISPLAY, fontWeight: 900, fontSize: 150, letterSpacing: '-0.05em',
              color: '#fff', textShadow: '0 12px 40px rgba(0,0,0,.6)', transform: `translateY(${(1 - q) * 105}%)`}}>{c}</span>;
          })}
        </div>
      </div>
      {/* progress dots */}
      <div style={{position: 'absolute', left: 0, right: 0, bottom: 64, display: 'flex', justifyContent: 'center', gap: 14}}>
        {items.map((m, i) => (
          <div key={i} style={{
            width: i === idx ? 54 : 12, height: 12, borderRadius: 6, background: i <= idx ? BRAND.ember : 'rgba(255,255,255,.25)',
            boxShadow: i === idx ? `0 0 20px ${BRAND.ember}` : 'none',
          }} />
        ))}
      </div>
      <Flash at={it.at} dur={0.18} peak={0.35} />
      {void nextAt}
    </AbsoluteFill>
  );
};

// ---------------------------------------------------------------- end card
export const EndCard: React.FC<{t0: number; total: number}> = ({t0, total}) => {
  const v = useV();
  const a = v - t0;
  if (a < 0) return null;
  const fade = 1 - clamp((v - (total - 0.85)) / 0.8);
  const push = 1 + a * 0.012;
  const lp = easeOutExpo(clamp(a / 0.6));
  return (
    <AbsoluteFill style={{background: '#000', opacity: fade}}>
      <AbsoluteFill style={{transform: `scale(${push})`}}><Backdrop dark={0.74} /></AbsoluteFill>
      {/* drifting embers */}
      {Array.from({length: 36}).map((_, i) => {
        const x = rnd(i) * W, sp = 20 + rnd(i + 7) * 60;
        const y = H + 40 - ((a * sp + rnd(i + 3) * H) % (H + 80));
        const s = 2 + rnd(i + 11) * 4;
        return <div key={i} style={{position: 'absolute', left: x + Math.sin(a + i) * 20, top: y, width: s, height: s, borderRadius: s,
          background: BRAND.ember2, opacity: 0.25 + rnd(i + 5) * 0.5, boxShadow: `0 0 ${s * 4}px ${BRAND.ember}`}} />;
      })}
      <AbsoluteFill style={{alignItems: 'center', justifyContent: 'center', flexDirection: 'column'}}>
        <div style={{display: 'flex', alignItems: 'center', gap: 34, transform: `translateY(${-70 + (1 - lp) * 40}px) scale(${lerp(1.15, 1, lp)})`, opacity: lp}}>
          <Glyph draw={easeOutExpo(clamp(a / 0.7))} lens={springStep(a - 0.15, 200, 13)} dot={springStep(a - 0.05, 260, 14)} size={170} glow={0.4 + 0.3 * Math.sin(a * 2.2)} />
          <div style={{fontFamily: DISPLAY, fontWeight: 800, fontSize: 150, letterSpacing: '-0.05em', color: '#fff'}}>RustCast</div>
        </div>
        <div style={{marginTop: -30}}>
          <Words text="Free. Open source. Built in Rust." t0={t0 + 0.7} size={50} weight={600} accent={['Open', 'source']} color="rgba(255,255,255,.85)" tracking="-0.015em" />
        </div>
        <div style={{
          marginTop: 54, display: 'flex', alignItems: 'center', gap: 18, padding: '20px 34px', borderRadius: 999,
          background: 'rgba(255,255,255,.06)', border: '1px solid rgba(255,255,255,.16)', backdropFilter: 'blur(18px)',
          opacity: easeOutCubic(clamp((a - 1.4) / 0.5)), transform: `translateY(${(1 - easeOutBack(clamp((a - 1.4) / 0.5))) * 30}px)`,
          boxShadow: `0 0 ${40 + 30 * Math.sin(a * 2)}px rgba(242,84,45,.25)`,
        }}>
          <svg width="40" height="40" viewBox="0 0 24 24"><path fill="#fff" d="M12 .5a11.5 11.5 0 0 0-3.64 22.41c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.53-1.33-1.28-1.69-1.28-1.69-1.05-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.77 2.71 1.26 3.37.96.1-.75.4-1.26.73-1.55-2.56-.29-5.25-1.28-5.25-5.68 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.77 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.41-2.69 5.39-5.26 5.67.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 12 .5Z" /></svg>
          <span style={{fontFamily: MONO, fontSize: 40, color: '#fff', letterSpacing: '-0.01em'}}>
            github.com/<span style={{color: BRAND.ember2}}>shashank03-dev</span>/rustcast
          </span>
        </div>
        <div style={{marginTop: 34, fontFamily: UI, fontSize: 30, color: 'rgba(255,255,255,.55)', letterSpacing: '0.02em',
          opacity: easeOutCubic(clamp((a - 2.6) / 0.6))}}>
          ★ Star it · Fork it · Make it yours
        </div>
      </AbsoluteFill>
      <Flash at={t0} dur={0.4} peak={0.6} />
    </AbsoluteFill>
  );
};
void H; void SH; void SW; void lerp;
