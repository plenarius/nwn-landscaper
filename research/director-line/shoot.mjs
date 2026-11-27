import fs from 'node:fs';
import { chromium } from 'playwright';
const [what, area, plan, out, ...rest] = process.argv.slice(2);
const browser = await chromium.launch({ args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'] });
const page = await browser.newPage();
page.on('console', m => console.log('page:', m.text()));
page.on('pageerror', e => console.log('error:', e.message));
await page.goto('http://127.0.0.1:8765/preview.html');
await page.waitForFunction(() => window.setup && window.frame);
const save = (url, p) => fs.writeFileSync(p, Buffer.from(url.split(',')[1], 'base64'));
if (what === 'map') {
    const info = await page.evaluate(([a, p]) => window.setup(a, p, 640, 640), [area, plan]);
    console.log(JSON.stringify(info));
    save(await page.evaluate(() => window.map()), out);
} else {
    const info = await page.evaluate(([a, p]) => window.setup(a, p, 640, 360), [area, plan]);
    console.log(JSON.stringify(info));
    const times = rest.map(Number);
    for (const t of times) save(await page.evaluate(t => window.frame(t), t), `${out}-${String(t).padStart(2, '0')}.png`);
}
await browser.close();
