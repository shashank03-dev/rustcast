// Rasterise the RustCast mark with headless Chromium (Playwright).
//
//   node scripts/brand/render.mjs [--frames]
//
// Always writes the app icon set:
//   docs/icon.png                1024px master (embedded in the app)
//   assets/icons/rustcast.png    512px (tray, About dialog, window icon)
//   assets/icons/rustcast-N.png  16..256px (installed desktop icons);
//                                16-32px use the heavier small-size mark.
//   assets/icons/rustcast-glyph-{dark,light}.png  in-app glyph, 64px
// With --frames, also writes PNG frames of the motion mark to
// $FRAMES_DIR (default: ./target/brand-frames) for building previews.
//
// Needs Playwright (`npm i -g playwright`) and a Chromium it can launch.

import { chromium } from "playwright";
import { readFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const brand = join(root, "assets", "brand");
const read = (f) => readFileSync(join(brand, f), "utf8");

const browser = await chromium.launch(
  process.env.CHROMIUM_PATH ? { executablePath: process.env.CHROMIUM_PATH } : {},
);
const page = await browser.newPage({ deviceScaleFactor: 1 });

async function shoot(svg, size, out) {
  await page.setViewportSize({ width: size, height: size });
  await page.setContent(
    `<style>html,body{margin:0;background:transparent}svg{display:block;width:${size}px;height:${size}px}</style>${svg}`,
  );
  await page.locator("svg").screenshot({ path: out, omitBackground: true });
}

mkdirSync(join(root, "assets", "icons"), { recursive: true });
await shoot(read("rustcast-mark.svg"), 1024, join(root, "docs", "icon.png"));
await shoot(read("rustcast-mark.svg"), 512, join(root, "assets", "icons", "rustcast.png"));
for (const n of [16, 24, 32, 48, 64, 128, 256]) {
  const svg = read(n <= 32 ? "rustcast-mark-small.svg" : "rustcast-mark.svg");
  await shoot(svg, n, join(root, "assets", "icons", `rustcast-${n}.png`));
}

// The tile-less glyph drawn inside the launcher (dark and light UI).
for (const v of ["dark", "light"]) {
  await shoot(read(`rustcast-glyph-${v}.svg`), 64, join(root, "assets", "icons", `rustcast-glyph-${v}.png`));
}

if (process.argv.includes("--frames")) {
  const dir = process.env.FRAMES_DIR || join(root, "target", "brand-frames");
  const size = Number(process.env.FRAME_SIZE || 480);
  const fps = Number(process.env.FPS || 30);
  const ms = Number(process.env.DURATION_MS || 4200);
  mkdirSync(dir, { recursive: true });
  await page.setViewportSize({ width: size, height: size });
  await page.setContent(
    `<style>html,body{margin:0;background:#0d0d10}svg{display:block;width:${size}px;height:${size}px}</style>${read("rustcast-mark-animated.svg")}`,
  );
  const frames = Math.round((ms / 1000) * fps);
  for (let i = 0; i < frames; i++) {
    const t = (i * 1000) / fps;
    await page.evaluate((t) => {
      for (const a of document.getAnimations()) {
        a.pause();
        a.currentTime = t;
      }
    }, t);
    await page.screenshot({ path: join(dir, `f${String(i).padStart(4, "0")}.png`) });
  }
  console.log(`wrote ${frames} frames to ${dir}`);
}

await browser.close();
console.log("rendered icon set");
