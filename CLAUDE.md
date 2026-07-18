# CLAUDE.md — Forge Orchestrator

Universal orchestration engine for AI-powered development. Single Rust binary (`forge`) that decomposes specs into tasks, dispatches them to Claude/Codex/Gemini, and reconciles results.

## Quick Reference

```bash
# Build
cargo build --release          # Optimized binary (4 MB, LTO + strip)
cp target/release/forge ~/.local/bin/forge-orca

# Test
cargo test                     # All 378 tests (356 unit + 10 CLI + 12 MCP)
cargo clippy -- -W clippy::all # Lint

# Run
cargo run -- init              # Initialize .forge/ in a project
cargo run -- plan --generate   # Generate plan from SPEC.md
cargo run -- dashboard --pty   # TUI dashboard with native agent TUIs
cargo run -- mcp               # Start MCP server (stdio JSON-RPC 2.0)
cargo run -- status            # Task board + event log
cargo run -- uat               # Interactive UAT TUI
```

**IMPORTANT:** Stop all `forge-orca` processes before `cp` — "Text file busy" error otherwise.
**GPG:** Use `--no-gpg-sign` for commits in this repo.

## Architecture

```
src/
├── main.rs              # Entry: tokio + clap CLI (13 subcommands)
├── cli/                 # Command handlers (init, plan, run, start, dashboard, status,
│                        #   sync, mcp, config, verify, uat)
├── core/                # Deterministic engine
│   ├── state.rs         #   .forge/state.json — project metadata, auth modes, locks
│   ├── task.rs          #   Task lifecycle — Pending→InProgress→Completed/Failed
│   ├── event.rs         #   .forge/events.jsonl — append-only audit trail
│   ├── governance.rs    #   5-dimension health score + drift detection
│   ├── knowledge.rs     #   .forge/knowledge/ — research/decisions/learnings/patterns
│   ├── finding.rs       #   UAT findings with severity classification
│   └── plan.rs          #   .forge/plan.md + plan.yaml
├── adapters/            # AI tool adapters (Claude, Codex, Gemini)
│   ├── mod.rs           #   ToolAdapter trait (render_config, build_command, ready_pattern)
│   ├── claude.rs        #   Writes CLAUDE.md, smart --allowedTools by task type
│   ├── codex.rs         #   Writes AGENTS.md, codex --task-mode
│   └── gemini.rs        #   Writes GEMINI.md, gemini CLI
├── mcp/                 # MCP server (stdio transport, 10 tools)
│   ├── server.rs        #   JSON-RPC 2.0 dispatcher
│   ├── tools.rs         #   Tool implementations
│   └── protocol.rs      #   Type definitions
├── tui/                 # Terminal UI (ratatui + crossterm)
│   ├── app.rs           #   Dashboard state + execution engine (largest module)
│   ├── ui.rs            #   Rendering (task board, agent panes, event log)
│   ├── pty_session.rs   #   PTY allocation + vt100 terminal emulator
│   ├── uat_app.rs       #   UAT TUI state machine
│   └── uat_ui.rs        #   UAT TUI rendering
├── brain/               # Pluggable LLM backends
│   ├── mod.rs           #   ForgeBrain trait
│   ├── rule_based.rs    #   Free tier (heuristics, no LLM)
│   └── openai.rs        #   GPT-4o/o3/o4-mini via OpenAI API
└── detect/              # Tool detection (scan PATH for claude, codex, gemini)
```

## MCP Tools (11)

forge-orchestrator exposes these tools via `forge mcp` (stdio JSON-RPC 2.0):

| Tool | Purpose |
|------|---------|
| `forge_get_tasks` | List tasks (filter by status) |
| `forge_claim_task` | Claim task for an agent (locks files) |
| `forge_complete_task` | Mark task done (unlocks files, logs event) |
| `forge_get_state` | Full orchestration state |
| `forge_get_plan` | Master plan (plan.md) |
| `forge_capture_knowledge` | Capture learning (auto-classifies) |
| `forge_get_knowledge` | Query knowledge base |
| `forge_check_drift` | Compare work vs. vision (SPEC.md) |
| `forge_get_health` | Comprehensive governance health check |
| `forge_set_project` | Switch active project |
| `forge_get_events` | Query event history (count, task_id, event_type filters) |

## Task Model

**Phases:** Build → Verify → Fix (auto-transitions in dashboard)
**Statuses:** Pending → Assigned → InProgress → Completed / Failed / Blocked
**ID Prefixes:** T-xxx (build), V-xxx (verify), F-xxx (fix)

Tasks have: `assigned_to` (agent), `locked_files` (exclusive access), `depends_on` (blocking), `task_type` (design/implement/review/test/document), `acceptance_criteria`.

## Adapter Pattern

Each AI tool has an adapter implementing `ToolAdapter`:

| Method | Purpose |
|--------|---------|
| `render_config()` | Write context file (CLAUDE.md, AGENTS.md, GEMINI.md) |
| `build_command()` | CLI command for headless execution |
| `build_command_interactive()` | CLI command for PTY/TUI mode |
| `initial_input()` | Text typed into TUI after startup |
| `ready_pattern()` | Pattern indicating TUI ready for input |

**Ready patterns:** Claude=`"to cycle"`, Codex=`"help you with"`, Gemini=`"Type your message"`

### Stargate Auto-Approve Contract (MANDATORY)

Every adapter's `build_command_interactive()` MUST pass the tool's auto-approve flag unconditionally in Stargate mode. Agents inside dashboard PTY panes run unattended — they cannot wait for human approval prompts.

| Tool | Auto-Approve Flag | Status |
|------|------------------|--------|
| Claude | `--dangerously-skip-permissions` | Working (v1.3.0+) |
| Codex | `codex exec --full-auto --skip-git-repo-check "prompt"` | Fixed (v1.3.2) |
| Gemini | `--yolo --sandbox=false` | Fixed (v1.3.2) |

**Rule**: When adding a new adapter, the first thing to verify is its unattended-execution flag. If the CLI tool has no such flag, it cannot be used in Stargate mode.

## PTY / Stargate Mode

`--pty` flag launches native agent TUIs inside the dashboard:
- **vt100 crate** for full terminal emulation (cursor, scroll, alt-screen)
- **Ready-pattern detection** polls every 200ms, 300ms grace, text→500ms→Enter
- **Completion detection:** signal file (`.forge/signals/T-xxx.complete`) or heuristic (pattern disappear→reappear + 10s min + 3s quiet)
- Text and Enter MUST be separate PTY writes

## .forge/ Directory

```
.forge/
├── state.json       # Live state (auth modes, locks, scheduler config)
├── plan.md / .yaml  # Master plan (human + machine readable)
├── events.jsonl     # Append-only audit log
├── tasks/           # T-xxx.md, V-xxx.md, F-xxx.md
├── knowledge/       # decisions/, learnings/, research/, patterns/
├── findings/        # UAT findings (JSON)
├── signals/         # Completion signal files (T-xxx.complete)
└── uat-status.json  # Persistent UAT pass/fail state
```

## Cross-Repo Integration

```
forge-plugin (Claude Code)  ──stdio MCP──►  forge-orchestrator (this repo, 10 tools)
forge-plugin                ──spawns──►     forge-ui (http://localhost:5050)
forge-orchestrator          ──headless──►   runs CLI or TUI dashboard independently
```

- **forge-plugin** (`../forge-plugin/`): Claude Code plugin with 21 commands, 22 agents, 29 skills. Calls this repo's MCP tools via stdio.
- **forge-ui** (`../v3/`): React dashboard + Infinity Terminal. Visual layer, no code dependency on this repo.
- **MCP is the only integration layer.** No direct imports or shared code between repos.

## Common Mistakes

- Forgetting to stop dashboard before deploying binary ("Text file busy")
- Sending `\n` in PTY input (use spaces — `\n` triggers multiline mode in Claude Code TUI)
- Indexing `self.tasks` by `selected_index` after separate sorting (sort in `reload_tasks()`, not render)
- Calling `schedule_unblocked_tasks()` only on events — also needed in `handle_tick()` for pacing
- Testing ready-pattern without `pattern_disappeared` guard (causes false completion)

## Key Dimensions

- **Language:** Rust 1.93.0, Edition 2024
- **Binary:** ~4 MB (release, LTO, stripped)
- **Tests:** 378 (356 unit + 10 CLI + 12 MCP integration)
- **Version:** 1.5.2
- **Repo:** github.com/nxtg-ai/forge-orchestrator

## ASIF Governance

This project is part of NXTG-Forge (P-03) in the ASIF portfolio (Developer Tools vertical).

On every session (check BOTH your own NEXUS and the program NEXUS):
1. Read `.asif/NEXUS.md` (your own) — check for `## CoS Directives` section
1a. ALSO read `../.asif/NEXUS.md` (program-level) — check for directives targeting this repo
2. Execute any PENDING directives before other work (unless Asif overrides)
3. Write your response inline under each directive
4. Update initiative statuses in NEXUS if your work changes them
5. If you have questions for the CoS, add them under `## Team Questions` in NEXUS


## Release Protocol Enforcement (ASIF Standard, ADR-036)

When you bump the version in `Cargo.toml`:
1. **Tag**: `git tag vX.Y.Z && git push origin vX.Y.Z`
2. **GH Release**: `gh release create vX.Y.Z --notes-from-tag` (attach the musl binary)
3. **Publish**: `cargo publish` (if/when crates.io publishing is enabled)
4. **CHANGELOG**: roll `[Unreleased]` → `[vX.Y.Z] — YYYY-MM-DD` in CHANGELOG.md
5. **Docs**: update any pinned version references in README.md / docs

Pre-push hook (Layer 1, `.git/hooks/pre-push`) chains CI Gate then runs the release-protocol gate — blocks pushes with a version bump but no git tag or dated CHANGELOG section. The daily `release-protocol-check.yml` workflow (Layer 2) opens a `release-drift` issue and auto-closes on resolve. Wolf's nightly sense pass surfaces drift portfolio-wide via `===SECTION:RELEASE_DRIFT===`.

**Bypass (EMERGENCY ONLY)**: `git push --no-verify` — and document the bypass in NEXUS or HANDOFF.
## Dx3 Brain Integration
On every session start, recall relevant context from Dx3 before starting work:
- Use recall() to check for prior decisions, lessons, and patterns related to your current task
- After shipping work, use remember() to store what you learned
- The brain at dx3-cognitive MCP has context from ALL projects — use it

This is how the portfolio compounds intelligence. Your work benefits from every other team's learning.

## Voice Identity

**Voice**: `am_onyx`
**Service**: http://100.123.83.34:8880/v1/audio/speech
**Registry**: ~/ASIF/standards/voice-registry.md
**Use**: every cycle-complete, every P0/P1 completion, every directive response.

Speak via: `~/ASIF/scripts/cos-speak-remote --voice am_onyx "text"`

<!-- ASIF:TEAM-ALIGNMENT-WIRING:START -->
## ASIF Alignment Wiring

@/home/axw/ASIF/standards/claude-team-alignment-wiring.md

- Team alignment id: `forge`.
- Cross-team room: `/alignment`, written through `~/ASIF/scripts/alignment-say`.
- If an `[ALIGNMENT ...]` message appears, respond through `alignment-say`; do not answer only in this private TUI.
- Deterministic state first: typed Dx3/asifctl, `.asif/NEXUS.md`, git/tests/runtime probes. Prose is backup and local steering only.
<!-- ASIF:TEAM-ALIGNMENT-WIRING:END -->
