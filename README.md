# agentwatch

A local web dashboard for watching your [Claude Code](https://claude.com/claude-code) sessions and subagents, from any
folder, without getting in the way of the work.

- **Zero tokens.** It only reads the transcript files Claude Code already writes under `~/.claude/projects/`.
- **Zero dependencies.** Plain Node 18+. Listens on `127.0.0.1` only. Read-only.
- **Session agnostic.** One dashboard shows every project and session; filter by project or pick a session.
- **Browser-AI friendly.** A server-rendered *Collection page* (`/collection`) and a plain-text snapshot give an AI
  assistant attached to your browser everything it needs to advise you, summarise progress, or explain what your
  subagents are doing, so the assistant doesn't have to interrupt your session.

## Install

```sh
git clone https://github.com/coreylallojr/agentwatch.git
cd agentwatch
npm link            # puts an `agentwatch` command on your PATH (works in PowerShell, cmd, bash)
```

or straight from GitHub: `npm install -g github:coreylallojr/agentwatch`

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
  `isError`, `deny`, `stuck`).

## Privacy

Everything stays on your machine. All displayed text is passed through a redactor that masks emails, SSNs, card numbers
(Luhn-checked), JWTs, bearer tokens, `*_TOKEN=` / `*_SECRET=` style values and long hex/base64 secrets. Redaction is
best effort; treat the dashboard as sensitive anyway.

## Configuration

| Variable | Meaning |
|---|---|
| `CLAUDE_PROJECTS_DIR` | Where Claude Code transcripts live (default `~/.claude/projects`) |
| `AGENTWATCH_HOME` | Where agentwatch keeps its pidfile and optional `rules.json` (default `~/.agentwatch`) |

## Develop

```sh
npm test      # node --test; the headless-browser test is skipped when Playwright is not installed
```

## License

MIT
