# agentwatch

A local dashboard, and a set of AI-agent tools, for watching your [Claude Code](https://claude.com/claude-code) sessions
and subagents, from any folder, without getting in the way of the work.

- **Zero tokens.** It only reads the transcript files Claude Code already writes under `~/.claude/projects/`.
- **One small binary.** Written in Rust, no runtime needed. It listens on `127.0.0.1` only and never changes your transcripts.
- **Light.** About 0.8% of one CPU core while idle, about 50 MB of memory, and live updates within about 100 ms.
- **Session agnostic.** One dashboard shows every project and session; filter by project or pick a session.
- **For AI agents too.** An MCP server and two skills let Claude check on its own subagents: wait for them instead of
  polling, read a compact timeline, see why something failed.
- **Grows with you.** Agents record what agentwatch couldn't do for them; the `agentwatch-evolve` skill turns those
  ideas into tested changes on your own fork.

## Install

**Windows** (PowerShell):

```powershell
irm https://raw.githubusercontent.com/coreylallojr/agentwatch/main/install.ps1 | iex
```

**macOS / Linux**:

```sh
curl -fsSL https://raw.githubusercontent.com/coreylallojr/agentwatch/main/install.sh | sh
```

The installer downloads the latest release for your platform, checks its SHA-256, puts `agentwatch` on your PATH and
runs `agentwatch setup`. Setup installs the skills into `~/.claude/skills` and registers the MCP server with Claude Code
at user scope. Restart Claude Code afterwards. Set `AGENTWATCH_NO_SETUP=1` to skip setup.

**As a Claude Code plugin** (bundles the skills and the MCP server; the `agentwatch` binary must be on your PATH, so run
an installer above with `AGENTWATCH_NO_SETUP=1` first):

```
/plugin install agentwatch --marketplace coreylallojr/agentwatch
```

**From source**: `cargo install --git https://github.com/coreylallojr/agentwatch agentwatch`, then `agentwatch setup`.

## Use

```sh
agentwatch              # open the dashboard, focused on the project for the current folder
agentwatch --all        # open it with every project visible
agentwatch status       # list active sessions and subagents in the terminal (no browser, no server)
agentwatch snapshot     # print the plain-text collection snapshot (pipe it to an AI)
agentwatch stop         # stop the background server
agentwatch --no-open    # start the server without opening a browser
agentwatch --port 4800  # default port is 4778
```

The first run starts a small background server. It exits by itself after 30 minutes with no browser connected and no
transcript activity.

## What you see

- **Lanes**: one per subagent. Status is running, thinking, stale (idle > 90 s), stuck (idle > 240 s) or done.
- **Timeline**: assistant text as chat bubbles; each tool call is an expandable card with its input and trimmed output.
- **Collection page** (`/collection`): every agent and every event, fully expanded and server-rendered, with a built-in
  "Watchkeeper" protocol that tells a browser AI how to read the page and what to report.
- **Focus rules**: highlight or desktop-toast notable events (browser automation, hook/guard denials, stuck agents).
  Put your own in `~/.agentwatch/rules.json` (see `rules.json` in this repo for the format; keys are `tool`, `path`,
  `isError`, `deny`, `stuck`). Patterns use Rust regex syntax, which has no lookarounds.

## For AI agents

`agentwatch setup` (or the plugin) gives Claude Code these MCP tools. All of them are read-only over your transcripts
and return compact, redacted text:

| Tool | Use it to |
|---|---|
| `agentwatch_wait` | block until a subagent, any subagent or all subagents finish (or change, or get stuck) instead of polling; returns their final handbacks |
| `agentwatch_status` | list every subagent of the current session with status, counts and latest activity |
| `agentwatch_agent` | read one agent's prompt, numbered timeline and final handback (paged, noise filter) |
| `agentwatch_event` | read one tool call's full input and output |
| `agentwatch_errors` | list failures classified as recovered, unresolved, unknown or blocked-by-guard |
| `agentwatch_sessions` | find recent sessions across projects |
| `agentwatch_snapshot` | get the whole session as text |
| `agentwatch_dashboard` | start the dashboard and return (or open) its URL |
| `agentwatch_suggest_improvement` | record something agentwatch couldn't do, in `~/.agentwatch/ideas.md` |

"Current session" means the latest session of the project Claude Code is running in (`CLAUDE_PROJECT_DIR`).

Two skills come with it:

- **`agentwatch`** tells Claude when to use these tools and how to keep them cheap.
- **`agentwatch-evolve`** takes an idea from `agentwatch ideas` (or your request), builds it on a branch of **your fork**,
  runs the tests and parity checks, and opens a pull request to your fork. It asks before creating the fork, before
  pushing and before opening the PR. Sending a change upstream is optional.

Run the MCP server yourself with `agentwatch mcp`, or register it by hand:
`claude mcp add --transport stdio --scope user agentwatch -- agentwatch mcp`.

## Privacy

Everything stays on your machine. All displayed text is passed through a redactor that masks emails, SSNs, card numbers
(Luhn-checked), JWTs, bearer tokens, `*_TOKEN=` / `*_SECRET=` style values and long hex/base64 secrets. Redaction is
best effort; treat the dashboard as sensitive anyway. The MCP tools apply the same redaction, and present transcript text
as observed data rather than instructions.

## Configuration

| Variable | Meaning |
|---|---|
| `CLAUDE_PROJECTS_DIR` | Where Claude Code transcripts live (default `~/.claude/projects`) |
| `AGENTWATCH_HOME` | Where agentwatch keeps its pidfile, optional `rules.json` and `ideas.md` (default `~/.agentwatch`) |
| `AGENTWATCH_NO_SETUP` | Installers: skip `agentwatch setup` |

## Develop

The Rust crate is in `rust/`. The original Node implementation (`src/`, `bin/`) stays in the repo as the **parity
oracle**: the Rust output must match it byte for byte.

```sh
cargo test --manifest-path rust/Cargo.toml
cargo build --release --manifest-path rust/Cargo.toml
npm test                                    # Node suite
now=$(node parity/fixtures.mjs .parity-fx)  # synthetic projects dir
node parity/run.mjs  --projects .parity-fx --now "$now" --bin rust/target/release/agentwatch   # every API surface
node parity/http.mjs --projects .parity-fx --bin rust/target/release/agentwatch                # live HTTP, route by route
node parity/live.mjs --bin rust/target/release/agentwatch    # live updates while files change
node parity/mcp.mjs  --bin rust/target/release/agentwatch    # MCP server end to end
node parity/bench.mjs                                        # Node vs Rust: latency, memory, idle CPU
```

Pushing a `v*` tag builds release binaries for Windows, Linux and macOS and publishes them with checksums
(`.github/workflows/release.yml`). The tag must match the version in `rust/Cargo.toml`.

Known differences from the Node version: a JS string slice that splits an emoji yields U+FFFD; non-ISO timestamps are not
parsed; `rules.json` patterns use Rust regex syntax.

## License

MIT
