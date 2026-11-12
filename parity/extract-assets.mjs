// Regenerates rust/assets/* from the Node implementation (the static parts of the Collection page).
// node parity/extract-assets.mjs [--check]   (--check exits 1 when the committed assets are out of date)
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildModel, renderPage, renderBody, WATCHKEEPER_PROTOCOL } from '../src/collection.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const m = buildModel({ agents: [], sessionIds: [], sessions: [], staleSec: 90, stuckSec: 240 }, {}, { now: 1 });
const mm = { ...m, version: '@@VERSION@@', schema: '@@SCHEMA@@' };
const body = renderBody(mm);
const NOW = new Date(1).toISOString();
const page = renderPage(mm);
const i = page.indexOf(body);
if (i < 0 || page.indexOf(body, i + 1) >= 0) throw new Error('body not found exactly once');
const shell = (page.slice(0, i) + '@@BODY@@' + page.slice(i + body.length)).split(NOW).join('@@NOW@@');
const out = { 'collection_page.html': shell, 'watchkeeper.txt': WATCHKEEPER_PROTOCOL };
let stale = false;
for (const [f, text] of Object.entries(out)) {
  const p = path.join(ROOT, 'rust', 'assets', f);
  const cur = fs.existsSync(p) ? fs.readFileSync(p, 'utf8') : null;
  if (cur === text) continue;
  stale = true;
  if (!process.argv.includes('--check')) fs.writeFileSync(p, text);
}
if (process.argv.includes('--check') && stale) { console.error('rust/assets out of date: run node parity/extract-assets.mjs'); process.exit(1); }
