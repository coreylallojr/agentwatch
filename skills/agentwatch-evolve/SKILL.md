---
name: agentwatch-evolve
description: Turn recorded agentwatch improvement ideas (~/.agentwatch/ideas.md) or a user's request into a tested change on the user's own fork of agentwatch, then open a pull request to that fork. Use when the user says "improve agentwatch", "build that agentwatch idea", "add X to agentwatch", or asks what agentwatch ideas are pending.
---

# agentwatch-evolve

agentwatch grows from use: agents record gaps with `agentwatch_suggest_improvement`, and this skill turns them into
changes on **the user's own fork**. The user owns the fork. Sending a change upstream is optional and happens only if they ask.

## Ground rules

- **Outward-facing steps need the user's explicit yes, every time:** creating a fork, pushing a branch, opening a PR.
  Ask in plain words ("Push branch `improve/x` to yourname/agentwatch and open a PR to your fork's main?") and wait.
- **Never put transcript content, personal data or secrets** into code, test fixtures, commits, issues or PRs.
  Fixtures are synthetic (see `parity/fixtures.mjs`).
- **Commit as the user**, with their configured git identity. Don't add a co-author line unless they want one.
- **One idea per branch.** Keep the change as small as the idea allows.

## 1. Pick the work

1. Read the ideas: `agentwatch ideas` (or `~/.agentwatch/ideas.md`). Entries headed `## [open]` are pending.
2. Summarize the open ideas for the user in one line each, recommend one, and confirm before building.
   An idea can also come straight from the user's request.

## 2. Get a working copy of the user's fork

1. Look for an existing clone: ask the user, or check common places (`~/agentwatch`, `~/Documents/agentwatch`, `~/src/agentwatch`),
   and confirm with `git remote -v`. `origin` should be the user's fork.
2. If there is none: `gh auth status` must succeed. Then, **after the user agrees**, run `gh repo fork coreylallojr/agentwatch --clone`.
   That creates the fork under their GitHub account and adds `upstream`.
3. Sync it: `git fetch upstream && git checkout main && git merge --ff-only upstream/main`. If that can't fast-forward, stop and ask.
4. Branch: `git checkout -b improve/<short-slug>`.

## 3. Know the codebase

| Path | What |
|---|---|
| `rust/src/parse.rs` | transcript parsing, redaction, tool-call summaries, status, focus rules |
| `rust/src/store.rs` | session discovery, incremental tail-reading, state queries |
| `rust/src/collection.rs` | the server-rendered Collection page and text snapshot |
| `rust/src/server.rs` | HTTP + server-sent events, file watcher, timers |
| `rust/src/mcp.rs` | the MCP server: tool catalogue, `INSTRUCTIONS`, tool handlers |
| `rust/src/cli.rs` | the `agentwatch` command line |
| `src/index.html` | the dashboard UI (embedded into the binary) |
| `src/*.mjs` | the original Node implementation, kept as the **parity oracle** |
| `parity/` | `run.mjs` (byte-for-byte API/page comparison), `http.mjs` (live HTTP), `live.mjs` (live updates), `bench.mjs` (performance), `fixtures.mjs` (synthetic data) |
| `skills/` | these skills (embedded into the binary; `agentwatch setup` installs them) |

**Prefer additive changes.** A new MCP tool, CLI command, query parameter or route doesn't touch existing output, so parity
still holds. If the idea changes existing output (a page, an API response, a summary line), make the same change in
`src/*.mjs` so the oracle agrees. If that's impractical, say in the PR which surfaces intentionally diverge from Node.

## 4. Build and prove it

Run from the repo root (Rust stable and Node 18+):

```
cargo test --manifest-path rust/Cargo.toml
cargo build --release --manifest-path rust/Cargo.toml
npm test                                                          # Node suite, still the oracle
node parity/fixtures.mjs .parity-fx > .parity-now                 # synthetic projects dir
node parity/run.mjs --projects .parity-fx --now <contents of .parity-now> --bin rust/target/release/agentwatch
node parity/http.mjs --projects .parity-fx
node parity/live.mjs
```

(On Windows the binary is `agentwatch.exe`.)

- Add a test for the new behaviour: a unit test in the module, or an integration test in `rust/tests/port.rs`.
  For new MCP tools, add a case that calls the handler, and check that `tools()` stays well-formed.
- For performance-motivated ideas, run `node parity/bench.mjs` before and after, and put both numbers in the PR.
- All of the above must pass, or the PR must explain each remaining difference.

## 5. Ship it to the fork

1. Commit with a message that states the idea, the change and the evidence (tests, parity, benchmark).
2. In `~/.agentwatch/ideas.md`, change the idea's heading from `## [open]` to `## [done]` and add the branch name.
   This file is local and not part of the repo.
3. **Ask**, then push: `git push -u origin improve/<slug>`.
4. **Ask**, then open the PR **against the user's fork**:
   `gh pr create --repo <user>/agentwatch --base main --head improve/<slug> --title "..." --body "..."`.
   The body says what the idea was, what changed, how it was tested, and any parity divergence.
5. Once it's merged into their fork, update the local install:
   `cargo install --path rust --force`, then `agentwatch setup` (refreshes the skills), then restart Claude Code.

## 6. Upstream (only if the user asks)

If the user wants to share the improvement: `gh pr create --repo coreylallojr/agentwatch --base main --head <user>:improve/<slug>`.
Upstream PRs must keep full parity or include the matching `src/*.mjs` change.
