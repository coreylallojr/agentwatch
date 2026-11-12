// Node side of the parity harness: the same call sequence as `agentwatch __dump`, with Date.now pinned.
// node parity/oracle.mjs --projects <dir> --now <ms>
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const val = (n) => { const i = process.argv.indexOf(n); return i >= 0 ? process.argv[i + 1] : null; };
const NOW = Number(val('--now'));
Date.now = () => NOW;
const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const { Store } = await import('../src/server.mjs');
const { buildModel, renderPage, renderBody, snapshotText } = await import('../src/collection.mjs');
const { compileRules } = await import('../src/parse.mjs');

const rules = compileRules(JSON.parse(fs.readFileSync(path.join(ROOT, 'rules.json'), 'utf8')));
const st = new Store({ projectsRoot: val('--projects'), rules });
await st.scan();
const out = {};
const projects = (await st.state({ session: 'all-recent', main: false })).projects.map((p) => p.key);
const scopes = [['latest', null], ['all-recent', null]];
for (const p of projects) { scopes.push(['latest', p]); scopes.push(['all-recent', p]); }
for (const s of st.sessions.values()) scopes.push([s.id, null]);
for (const [sess, proj] of scopes) {
  for (const main of [false, true]) out[`state|${sess}|${proj ?? ''}|${main}`] = JSON.stringify(await st.state({ session: sess, main, project: proj }));
  const s2 = await st.state({ session: sess, main: true, project: proj });
  const details = Object.fromEntries(s2.agents.map((a) => [a.id, st.agent(a.id, null, { full: true })]));
  const m = buildModel(s2, details, { now: NOW });
  const key = `${sess}|${proj ?? ''}`;
  out[`page|${key}`] = renderPage(m);
  out[`fragment|${key}`] = renderBody(m);
  out[`snapshot|${key}`] = snapshotText(m);
}
for (const rec of st.files.values()) {
  const id = rec.id;
  out[`agent|${id}`] = JSON.stringify(st.agent(id, null) ?? null);
  out[`agentfull|${id}`] = JSON.stringify(st.agent(id, null, { full: true }) ?? null);
  for (let i = 0; i < rec.builder.events.length; i++) out[`event|${id}|${i}`] = JSON.stringify(st.agent(id, i) ?? null);
}
process.stdout.write(JSON.stringify(out));
