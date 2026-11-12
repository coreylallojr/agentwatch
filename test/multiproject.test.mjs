import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { Store } from '../src/server.mjs';

function session(root, key, cwd, sid, withAgent) {
  const dir = path.join(root, key);
  fs.mkdirSync(path.join(dir, sid, 'subagents'), { recursive: true });
  fs.writeFileSync(path.join(dir, `${sid}.jsonl`), JSON.stringify({ type: 'user', cwd, timestamp: new Date().toISOString(), message: { content: 'hi' } }) + '\n');
  if (withAgent) {
    fs.writeFileSync(path.join(dir, sid, 'subagents', 'agent-a1.jsonl'), JSON.stringify({ type: 'user', agentId: 'a1', timestamp: new Date().toISOString(), message: { content: 'go' } }) + '\n');
    fs.writeFileSync(path.join(dir, sid, 'subagents', 'agent-a1.meta.json'), JSON.stringify({ agentType: 'Explore', description: 'look around' }));
  }
}

test('one store sees sessions from every project and can filter by project', async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'aw-multi-'));
  try {
    session(root, 'c--work-alpha', 'C:/work/alpha', 'aaaaaaaa-0000-0000-0000-000000000001', true);
    session(root, 'c--work-beta', 'C:/work/beta', 'bbbbbbbb-0000-0000-0000-000000000002', false);
    const store = new Store({ projectsRoot: root });
    await store.scan();
    const all = await store.state({ session: 'all-recent' });
    assert.deepEqual(all.projects.map((p) => p.name).sort(), ['alpha', 'beta']);
    assert.equal(all.sessions.length, 2);
    const alpha = await store.state({ session: 'latest', project: 'c--work-alpha' });
    assert.equal(alpha.selected, 'aaaaaaaa-0000-0000-0000-000000000001');
    assert.equal(alpha.agents.length, 1);
    assert.equal(alpha.agents[0].type, 'Explore');
    const beta = await store.state({ session: 'latest', project: 'c--work-beta' });
    assert.equal(beta.agents.length, 0);
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
});
