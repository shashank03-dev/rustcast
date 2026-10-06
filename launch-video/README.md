# RustCast launch video — source

Everything used to make the 60-second launch video. The footage is real: RustCast
running on a headless Ubuntu-style X11 desktop, driven by a script that moves the
mouse and types the way a person does.

## `capture/` — recording the screen

| File | What it does |
|---|---|
| `start.sh` | Starts Xvfb (2560×1440), openbox with the `YaruOB` theme, picom (shadows and rounded corners), the wallpaper, the panels and RustCast |
| `panels.py` | Ubuntu 24.04-style top bar and left dock (GTK3, Yaru icons) |
| `wall2.py` | Generates the wave wallpaper |
| `chrome.sh` | Opens a demo page (`demo-pages/`) as an app window at a fixed position |
| `drive.py` | Human-like driver: curved, eased mouse paths, natural typing rhythm. Records with `ffmpeg x11grab` (pointer hidden) and logs every cursor position, click, key and marker against wall-clock time |
| `takes.py`, `takes2.py` | One function per feature take: `launcher`, `jev`, `snap`, `ocr`, `rec`, `clip` |
| `norm.py` | Converts a take to constant 60 fps and aligns its event log to the video |
| `analyze_song.py` | Tempo, beat grid and per-second energy of the soundtrack (librosa) |

```sh
python3 takes.py launcher && python3 norm.py launcher
```

## `edit/` — the edit (Remotion)

- `src/Main.tsx`: the cut. Every section lands on a downbeat of the song, and each
  take is split into speed-ramped segments (`seg(videoStart, videoEnd, takeStart, takeEnd)`).
- `src/Take.tsx`: Screen Studio-style playback. A smooth camera eases between zoom
  keyframes, and the real Yaru cursor is redrawn from the event log with click
  ripples and keycap overlays.
- `src/Graphics.tsx`: cold open, logo animation on the drop, kinetic titles, the
  screenshot tool strip, the recorder split screen, the beat-synced montage and the
  end card.
- `src/sync.json`: downbeats, kicks and vocal hits of the music cut, in video seconds.

`public/` is not committed: it holds the screen takes, cursors, sound effects and
the soundtrack, which is not redistributed here. Rebuild it with the capture scripts, put your own
`music.wav` in, then:

```sh
npm i
npx remotion render src/index.ts Launch out/rustcast-launch.mp4 --codec=h264 --crf=16
```
