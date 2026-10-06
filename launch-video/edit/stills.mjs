import {bundle} from '@remotion/bundler';
import {renderStill, selectComposition, openBrowser} from '@remotion/renderer';
import path from 'path';
const times = process.argv.slice(2).map(Number);
const serveUrl = await bundle({entryPoint: path.resolve('src/index.ts')});
const browser = await openBrowser('chrome', {browserExecutable: '/opt/pw-browsers/chromium_headless_shell-1194/chrome-linux/headless_shell'});
const comp = await selectComposition({serveUrl, id: 'Launch', puppeteerInstance: browser});
for (const t of times) {
  const frame = Math.round(t * 60);
  await renderStill({composition: comp, serveUrl, frame, output: `stills/s_${t.toFixed(2)}.png`, puppeteerInstance: browser, scale: 0.5});
  console.log('ok', t);
}
await browser.close({silent: true});
