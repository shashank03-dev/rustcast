import {bundle} from '@remotion/bundler';
import {renderStill, selectComposition, openBrowser} from '@remotion/renderer';
import path from 'path';
// usage: node rstill.mjs id:seconds:out.png ...
const serveUrl = await bundle({entryPoint: path.resolve('src/index.ts')});
const browser = await openBrowser('chrome', {browserExecutable: '/opt/pw-browsers/chromium_headless_shell-1194/chrome-linux/headless_shell'});
for (const arg of process.argv.slice(2)) {
  const [id, t, out] = arg.split(':');
  const comp = await selectComposition({serveUrl, id, puppeteerInstance: browser});
  await renderStill({composition: comp, serveUrl, frame: Math.min(comp.durationInFrames - 1, Math.round(Number(t) * comp.fps)), output: out, puppeteerInstance: browser});
  console.log('ok', out);
}
await browser.close({silent: true});
