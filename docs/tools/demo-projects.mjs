// Synthetic demo workspace for screenshots and the landing page. Fictional project, fictional people, no real data.
// node docs/tools/demo-projects.mjs <outDir>      (timestamps are relative to now, so statuses look live)
import fs from 'node:fs';
import path from 'node:path';

const out = path.resolve(process.argv[2] || '.demo-projects');
fs.rmSync(out, { recursive: true, force: true });
const NOW = Date.now();
const at = (secAgo) => new Date(NOW - secAgo * 1000).toISOString();
let u = 0;
const rec = (agentId, secAgo, o) => JSON.stringify({ uuid: `d${++u}`, ...(agentId ? { agentId } : {}), timestamp: at(secAgo), ...o });
const user = (a, s, content) => rec(a, s, { type: 'user', message: { role: 'user', content } });
const text = (a, s, t) => rec(a, s, { type: 'assistant', message: { content: [{ type: 'text', text: t }] } });
const think = (a, s) => rec(a, s, { type: 'assistant', message: { content: [{ type: 'thinking', thinking: '' }] } });
const call = (a, s, id, name, input) => rec(a, s, { type: 'assistant', message: { content: [{ type: 'tool_use', id, name, input }] } });
const res = (a, s, id, content, isError = false) => rec(a, s, { type: 'user', message: { content: [{ type: 'tool_result', tool_use_id: id, content, ...(isError ? { is_error: true } : {}) }] } });

const KEY = 'C--code-acme-shop';
const CWD = 'C:\\code\\acme-shop';
const SID = '5f3c2a9e-7b1d-4c8e-9a6f-2e4d1b8c7a30';
const dir = path.join(out, KEY);
const sub = path.join(dir, SID, 'subagents');
fs.mkdirSync(sub, { recursive: true });
const write = (f, lines) => fs.writeFileSync(f, lines.join('\n') + '\n');
const agent = (id, meta, lines) => { write(path.join(sub, `agent-${id}.jsonl`), lines); fs.writeFileSync(path.join(sub, `agent-${id}.meta.json`), JSON.stringify(meta)); };

// main session: the orchestrator fans out work
write(path.join(dir, `${SID}.jsonl`), [
  JSON.stringify({ type: 'summary', cwd: CWD, timestamp: at(900) }),
  user(null, 900, 'We need payment retries in checkout. Map the flow, write the tests, review PR #142 and smoke-test checkout in a browser.'),
  text(null, 880, 'Splitting this into four parallel agents.'),
  call(null, 875, 'm1', 'Agent', { description: 'Map the checkout flow', subagent_type: 'Explore', prompt: 'Map the checkout flow' }),
  call(null, 874, 'm2', 'Agent', { description: 'Write payment retry tests', subagent_type: 'general-purpose', prompt: 'Write tests' }),
  call(null, 873, 'm3', 'Agent', { description: 'Review PR #142', subagent_type: 'code-reviewer', prompt: 'Review PR 142' }),
  call(null, 872, 'm4', 'Agent', { description: 'Smoke-test checkout in a browser', subagent_type: 'e2e-tester', prompt: 'Smoke test' }),
  res(null, 870, 'm1', 'Async agent launched'), res(null, 870, 'm2', 'Async agent launched'), res(null, 870, 'm3', 'Async agent launched'), res(null, 870, 'm4', 'Async agent launched'),
  call(null, 860, 'w1', 'mcp__agentwatch__agentwatch_wait', { until: 'any_done', timeout_seconds: 600 }),
  res(null, 410, 'w1', '1 of 4 watched agent(s) finished. (waited 7m30s)\n\n- [done] Explore "Map the checkout flow" · id a1f3c9 · 14 tools · 0 errors'),
  user(null, 405, [{ type: 'text', text: '[Subagent hand-back] Checkout flow mapped: cart → address → payment → confirm. Retries belong in PaymentService.charge().' }]),
  text(null, 400, 'The explorer is done. Waiting for the tests and the review.'),
  call(null, 398, 'w2', 'mcp__agentwatch__agentwatch_wait', { until: 'any_done', timeout_seconds: 600 }),
]);

// 1. Explore: done, with a markdown report
agent('a1f3c9', { agentType: 'Explore', description: 'Map the checkout flow', toolUseId: 'm1', finished: true }, [
  user('a1f3c9', 868, 'Map the checkout flow in acme-shop: entry points, services, where payments are charged, existing retry logic.'),
  think('a1f3c9', 866),
  call('a1f3c9', 865, 'g1', 'Glob', { pattern: 'src/checkout/**/*.ts' }), res('a1f3c9', 864, 'g1', 'src/checkout/CartPage.tsx\nsrc/checkout/AddressStep.tsx\nsrc/checkout/PaymentStep.tsx\nsrc/checkout/ConfirmStep.tsx\nsrc/checkout/api.ts'),
  call('a1f3c9', 860, 'g2', 'Grep', { pattern: 'charge\\(', path: 'C:\\code\\acme-shop\\src', glob: '*.ts' }), res('a1f3c9', 858, 'g2', 'src/payments/PaymentService.ts:41:  async charge(order: Order) {\nsrc/checkout/api.ts:88:  return payments.charge(order)'),
  call('a1f3c9', 850, 'r1', 'Read', { file_path: 'C:\\code\\acme-shop\\src\\payments\\PaymentService.ts' }), res('a1f3c9', 849, 'r1', 'export class PaymentService {\n  constructor(private gateway: Gateway) {}\n  async charge(order: Order) {\n    const r = await this.gateway.charge(order.total, order.card)\n    if (!r.ok) throw new PaymentError(r.code)\n    return r\n  }\n}'),
  call('a1f3c9', 840, 'r2', 'Read', { file_path: 'C:\\code\\acme-shop\\src\\checkout\\api.ts', offset: 70, limit: 40 }), res('a1f3c9', 839, 'r2', '...'),
  call('a1f3c9', 820, 'g3', 'Grep', { pattern: 'retry|backoff', path: 'C:\\code\\acme-shop\\src' }), res('a1f3c9', 819, 'g3', 'No matches found'),
  call('a1f3c9', 800, 'r3', 'Read', { file_path: 'C:\\code\\acme-shop\\src\\payments\\Gateway.ts' }), res('a1f3c9', 799, 'r3', 'export interface Gateway { charge(amount: number, card: Card): Promise<ChargeResult> }'),
  text('a1f3c9', 412, '## Checkout flow\n\n1. **CartPage** → **AddressStep** → **PaymentStep** → **ConfirmStep**\n2. `PaymentStep` calls `api.submitOrder()`, which calls `PaymentService.charge()`.\n\n## Findings\n\n- There is **no retry logic** anywhere (`retry|backoff` has no matches).\n- `PaymentService.charge()` throws `PaymentError` on any non-ok result, including transient gateway codes.\n\n**Recommendation:** add retries with exponential backoff inside `PaymentService.charge()` for codes `GATEWAY_TIMEOUT` and `RATE_LIMITED` only.'),
]);

// 2. Tests: running, a failed test run recovered, a tool in flight
agent('b7e214', { agentType: 'general-purpose', description: 'Write payment retry tests', toolUseId: 'm2' }, [
  user('b7e214', 867, 'Write unit tests for retrying transient payment failures in PaymentService. Use vitest.'),
  call('b7e214', 860, 't1', 'Read', { file_path: 'C:\\code\\acme-shop\\src\\payments\\PaymentService.ts' }), res('b7e214', 859, 't1', 'export class PaymentService { ... }'),
  call('b7e214', 700, 't2', 'Write', { file_path: 'C:\\code\\acme-shop\\src\\payments\\PaymentService.test.ts', content: "describe('charge retries', () => {\n  it('retries GATEWAY_TIMEOUT up to 3 times', async () => { /* ... */ })\n  it('does not retry CARD_DECLINED', async () => { /* ... */ })\n})" }),
  res('b7e214', 699, 't2', 'File created successfully'),
  call('b7e214', 690, 't3', 'Bash', { command: 'npx vitest run src/payments' }),
  res('b7e214', 655, 't3', 'FAIL  src/payments/PaymentService.test.ts > charge retries > retries GATEWAY_TIMEOUT up to 3 times\nAssertionError: expected 1 call, received 3\n\nTests  1 failed | 1 passed (2)', true),
  think('b7e214', 650),
  text('b7e214', 640, 'The test asserted the wrong call count. Fixing the expectation.'),
  call('b7e214', 630, 't4', 'Edit', { file_path: 'C:\\code\\acme-shop\\src\\payments\\PaymentService.test.ts', old_string: 'toHaveBeenCalledTimes(1)', new_string: 'toHaveBeenCalledTimes(3)' }),
  res('b7e214', 629, 't4', 'The file has been updated.'),
  call('b7e214', 620, 't5', 'Bash', { command: 'npx vitest run src/payments' }),
  res('b7e214', 590, 't5', ' ✓ src/payments/PaymentService.test.ts (2 tests) 41ms\n\n Test Files  1 passed (1)\n      Tests  2 passed (2)'),
  call('b7e214', 120, 't6', 'Write', { file_path: 'C:\\code\\acme-shop\\src\\payments\\retry.test.ts', content: "it('backs off 200, 400, 800 ms', async () => { /* ... */ })" }),
  res('b7e214', 119, 't6', 'File created successfully'),
  call('b7e214', 4, 't7', 'Bash', { command: 'npx vitest run --coverage' }),
]);

// 3. Review: thinking
agent('c42d8b', { agentType: 'code-reviewer', description: 'Review PR #142', toolUseId: 'm3' }, [
  user('c42d8b', 866, 'Review PR #142 (adds Apple Pay to PaymentStep). Focus on error handling and accessibility.'),
  call('c42d8b', 850, 'v1', 'Bash', { command: 'git diff main...pr/142 --stat' }), res('c42d8b', 849, 'v1', ' src/checkout/PaymentStep.tsx | 64 ++++++++---\n src/payments/applePay.ts     | 112 +++++++++++++\n 2 files changed, 158 insertions(+), 18 deletions(-)'),
  call('c42d8b', 830, 'v2', 'Read', { file_path: 'C:\\code\\acme-shop\\src\\payments\\applePay.ts' }), res('c42d8b', 829, 'v2', 'export async function payWithApplePay(...) { ... }'),
  call('c42d8b', 800, 'v3', 'Read', { file_path: 'C:\\code\\acme-shop\\src\\checkout\\PaymentStep.tsx', offset: 1, limit: 120 }), res('c42d8b', 799, 'v3', '...'),
  text('c42d8b', 30, '`payWithApplePay` swallows `AbortError`, so a cancelled sheet looks like a successful payment. Checking the button labels next.'),
  think('c42d8b', 8),
]);

// 4. Browser smoke test: playwright calls (highlighted), a guard denial (toast rule), a screenshot
agent('d90a17', { agentType: 'e2e-tester', description: 'Smoke-test checkout in a browser', toolUseId: 'm4' }, [
  user('d90a17', 865, 'Smoke-test checkout on http://localhost:5173 with the test card. Do not touch production.'),
  call('d90a17', 840, 'p1', 'mcp__playwright__browser_navigate', { url: 'http://localhost:5173/cart' }), res('d90a17', 838, 'p1', 'Navigated to http://localhost:5173/cart'),
  call('d90a17', 830, 'p2', 'mcp__playwright__browser_click', { element: 'Checkout button', ref: 'e12' }), res('d90a17', 828, 'p2', 'Clicked "Checkout"'),
  call('d90a17', 820, 'p3', 'mcp__playwright__browser_type', { element: 'Card number', ref: 'e31', text: '4242 4242 4242 4242' }), res('d90a17', 818, 'p3', 'Typed into "Card number"'),
  call('d90a17', 810, 'p4', 'Bash', { command: 'npm run deploy -- --env production' }),
  res('d90a17', 809, 'p4', '[deploy guard] blocked: production deploys need a human. Use --env staging.', true),
  call('d90a17', 800, 'p5', 'mcp__playwright__browser_click', { element: 'Place order', ref: 'e40' }), res('d90a17', 795, 'p5', 'Clicked "Place order"'),
  call('d90a17', 790, 'p6', 'mcp__playwright__browser_take_screenshot', { filename: 'confirm.png' }),
  res('d90a17', 788, 'p6', [{ type: 'text', text: 'Saved confirm.png' }, { type: 'image', source: { type: 'base64', media_type: 'image/png', data: fs.readFileSync(new URL('./demo-confirm.png', import.meta.url)).toString('base64') } }]),
  text('d90a17', 600, 'Checkout works end to end with the test card: order #A-1042 confirmed. The production deploy was blocked by the deploy guard, as it should be.'),
]);
fs.writeFileSync(path.join(sub, 'agent-d90a17.meta.json'), JSON.stringify({ agentType: 'e2e-tester', description: 'Smoke-test checkout in a browser', toolUseId: 'm4', finished: true }));

// 5. An older agent that went quiet (stale)
agent('e5b0c2', { agentType: 'Plan', description: 'Plan the v2 API migration' }, [
  user('e5b0c2', 400, 'Draft a migration plan from /api/v1/orders to /api/v2/orders.'),
  call('e5b0c2', 390, 'q1', 'Read', { file_path: 'C:\\code\\acme-shop\\docs\\api-v2.md' }), res('e5b0c2', 389, 'q1', '# Orders API v2\n...'),
  call('e5b0c2', 150, 'q2', 'WebFetch', { url: 'https://example.com/acme/api-changelog', prompt: 'list breaking changes' }),
]);

process.stdout.write(out);
