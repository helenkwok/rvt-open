// node site-check.mjs SITE_DIR MODEL BROWSER...: serves SITE_DIR and the model (as /model.rvt, outside the site), opens the page in each
// browser (chromium, firefox, webkit), waits for the notebook's last result and checks the counts it shows against
// expected-basic-sample.json. One line per browser; exit 1 if any fails.
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { chromium, firefox, webkit } from 'playwright';

const [dir, model, ...names] = process.argv.slice(2);
const expected = JSON.parse(fs.readFileSync(new URL('expected-basic-sample.json', import.meta.url)));
const types = { '.html': 'text/html', '.js': 'text/javascript', '.mjs': 'text/javascript', '.css': 'text/css', '.wasm': 'application/wasm', '.json': 'application/json', '.whl': 'application/octet-stream', '.svg': 'image/svg+xml' };
const root = path.resolve(dir);
const server = http.createServer((req, res) => {
  let p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
  const file = p === '/model.rvt' ? model : path.join(root, p.endsWith('/') ? p + 'index.html' : p);
  if ((file !== model && !file.startsWith(root)) || !fs.existsSync(file) || fs.statSync(file).isDirectory()) { res.writeHead(404); return res.end('not found'); }
  res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream', 'content-length': fs.statSync(file).size, 'access-control-allow-origin': '*' });
  fs.createReadStream(file).pipe(res);
});
await new Promise((r) => server.listen(8765, '127.0.0.1', r));

let failed = false;
for (const name of names) {
  const t0 = Date.now();
  const browser = await { chromium, firefox, webkit }[name].launch();
  const page = await (await browser.newContext({ viewport: { width: 1280, height: 1800 } })).newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e).slice(0, 300)));
  const rec = { browser: name, version: browser.version() };
  try {
    await page.goto('http://127.0.0.1:8765/index.html', { waitUntil: 'load' });
    // The last visible result of the notebook: the parameters of the first wall.
    await page.getByText('instance parameters').first().waitFor({ timeout: 900000 });
    rec.seconds = (Date.now() - t0) / 1000;
    const counts = await page.getByText(/\d+ elements in \d+ categories/).first().textContent();
    const m = counts.match(/(\d+) elements in (\d+) categories/);
    rec.shown = `${m[1]} elements in ${m[2]} categories`;
    rec.ok = Number(m[1]) === expected.total && Number(m[2]) === expected.categories;
  } catch (e) {
    rec.ok = false;
    rec.error = String(e.message).split('\n')[0];
    rec.page = (await page.locator('body').innerText().catch(() => '')).slice(0, 1500);
  }
  rec.errors = errors;
  fs.mkdirSync('site-shots', { recursive: true });
  await page.screenshot({ path: `site-shots/notebook-01-${name}.png`, fullPage: true });
  console.log('SITE ' + JSON.stringify(rec));
  failed ||= !rec.ok;
  await browser.close();
}
server.close();
process.exit(failed ? 1 : 0);
