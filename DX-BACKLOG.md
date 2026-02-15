# Forge Orchestrator — DX Backlog

> From live dogfood sessions on voice-jib-jab project (2026-02-10/11).

## Open Items (8 remaining)

### DX-032: Standalone UAT TUI
- **Priority:** HIGH (v1.x)
- **Where:** New `src/tui/uat_app.rs` + `src/tui/uat_ui.rs`
- **Problem:** `forge uat` dumps ALL acceptance criteria from ALL completed tasks (including V-xxx duplicates) as an unreadable wall, then shows a bare `>` prompt. Unusable on real projects (100+ criteria on voice-jib-jab).
- **Solution:** Replace CLI REPL with a ratatui-based TUI: task selector (filters out V-xxx), focused criteria view per task, finding capture with auto-classification, pass/fail marking. Plus inline mode: `forge uat "description"` for quick one-shot capture.
- **Status:** Assignment written (NEXT-ASSIGNMENT.md), ready for Claudio.
- **Vision:** Near-term CLI/TUI capture. Medium-term: forge-ui command center becomes the UAT surface for web apps (split-pane testing). Long-term: forge-extension (browser co-pilot with visual evidence capture).

### DX-024: Forge Stargate — Embedded Interactive Agent TUIs
- **Priority:** VISION (v2.0)
- **Where:** New architecture — PTY bridge per agent pane
- **Problem:** Current agent panes show captured stdout text. The dream is embedding ACTUAL running TUIs (Claude Code's TUI, Gemini's TUI, Codex's TUI) inside forge dashboard panes.
- **Solution:** Instead of spawning headless `-p` processes, spawn interactive CLIs in PTY sessions. Each pane becomes a real terminal with input/output. User can tab between them and interact directly.
- **Why "Stargate":** It's a portal into each AI's universe. Three portals, one command center.
- **Technical:** `portable-pty` or `pty-process` crate for PTY allocation, pipe each PTY's output to a ratatui pane, forward keystrokes when pane is focused.
- **Prerequisite:** DX-023 (done) and DX-027 (done) — scrollable/focusable panes + shell panes prove the pattern.

### DX-028 Tier 2/3: Git Worktrees for Parallel Isolation
- **Priority:** FUTURE (v1.x)
- **Where:** `src/tui/app.rs` + new `src/core/git.rs`
- **Problem:** Tier 1 auto-commit (done in v1.0) commits to current branch. When 3 agents edit the same files in parallel, conflicts are possible.
- **Solution:**
  - **Tier 2:** Each agent gets a `git worktree`, commits independently, merges on completion
  - **Tier 3:** `forge config git.strategy single|worktree|branch` — user picks strategy
- **See full spec in git history (DX-028 original description)**

### DX-033: Subscription Pacing — Human-Like Task Delays
- **Priority:** HIGH (v1.x)
- **Where:** `src/core/scheduler.rs` (new) + `src/tui/app.rs`
- **Problem:** Automated orchestration fires tasks in 0-5 second intervals, burning through subscription quotas (45-225 messages/5h on Codex Team, ~225 on Claude Max) in under 2 hours. Interactive TUI usage naturally paces at 1-3 minutes between actions.
- **Solution:** For `auth_mode = "subscription"` (the default), insert a random delay of 64-179 seconds between consecutive task dispatches per provider. This simulates natural human reading/thinking time. `auth_mode = "api"` runs at full speed with no delays.
- **Context:** Discovered during SynApps dogfood — Codex hit `usage_limit_reached` after 24 tasks in 2h, Gemini hit `429 No capacity` after 2 tasks. See research: `ecosystem/forge/research/cli-subscription-gating-2026-02-15.md`
- **Asif's directive:** "This is the DEFAULT for subscription based authentication vs API... API will have the FULL SPEED approach."

### DX-034: Rate Limit Detection — Parse 429 Errors
- **Priority:** HIGH (v1.x)
- **Where:** `src/adapters/claude.rs`, `codex.rs`, `gemini.rs`
- **Problem:** Rate limit errors are currently treated as generic task failures. The orchestrator doesn't distinguish between "your code crashed" and "you hit a quota wall."
- **Solution:** Parse stderr/stdout for provider-specific rate limit signatures:
  - Codex: `usage_limit_reached` + `resets_at` timestamp
  - Gemini: `429 No capacity available` (server capacity, not quota)
  - Claude: `rate_limit_error` or HTTP 429
- Mark tasks as `rate_limited` (new status) instead of `failed`. Extract reset timestamps where available.

### DX-035: Exponential Backoff on Rate Limit
- **Priority:** HIGH (v1.x)
- **Where:** `src/core/scheduler.rs` + `src/tui/app.rs`
- **Problem:** When a provider hits rate limits, all remaining tasks for that provider fail immediately instead of waiting and retrying.
- **Solution:** On `rate_limited` status: pause that provider's task queue, apply exponential backoff (30s → 60s → 120s → 240s, max 10 min), retry the task. Show backoff countdown in dashboard. After 3 consecutive rate limits from same provider, pause that provider entirely and redistribute to others if rotation is enabled.
- **Prerequisite:** DX-034 (rate limit detection)

### DX-036: Provider Rotation — Optional Task Redistribution
- **Priority:** MEDIUM (v1.x)
- **Where:** `src/core/scheduler.rs` + `src/cli/config.rs`
- **Problem:** When one provider is rate-limited, its remaining tasks sit idle while other providers may have quota available.
- **Solution:** Optional config (`forge config scheduler.rotation enabled`) that redistributes blocked tasks to available providers when rate limits hit. NOT the default — user must opt in.
- **Asif's directive:** "The provider rotation seems like a stop gap, not a solution. But I do like the option. Perhaps we add as a configuration option for DX."

### DX-037: Quota Monitoring — Provider Usage in Status/Dashboard
- **Priority:** MEDIUM (v1.x)
- **Where:** `src/tui/ui.rs` + `src/cli/status.rs`
- **Problem:** No visibility into how much quota each provider has consumed or when rate limits will reset. User discovers limits only when tasks fail.
- **Solution:** Track messages/tasks dispatched per provider per 5-hour window. Show in `forge status` and dashboard footer: `Claude: 12/225 msgs (5h) | Codex: 43/45 msgs (5h) [NEAR LIMIT] | Gemini: 2/60 RPM`. Parse reset timestamps from 429 responses to show countdown.

## Completed Items (32 of 40)

| DX | Description | Version |
|----|-------------|---------|
| DX-001–008 | Init, plan, config, status fixes | v0.2.0 |
| DX-009 | Spinner / progress indicators (indicatif) | v0.2.2 |
| DX-010 | Full task table with dependencies in `forge status` | v1.0.0 |
| DX-011 | Headless autonomous mode (`forge run` no args) | v1.0.0 |
| DX-012 | Per-agent auth config (subscription/api) | v0.2.0 |
| DX-013 | Async execution via tokio | v0.2.0 |
| DX-014 | TUI dashboard with live agent panes | v0.3.0 |
| DX-015 | Yolo permissions mode | v0.2.0 |
| DX-016 | Smart Claude adapter (task-type-aware tools) | v0.2.1 |
| DX-017 | Codebase-aware plan generation (spec vs reality) | v0.2.2 |
| DX-018 | Rate limit backoff (exponential with jitter) | v1.0.0 |
| DX-019 | Gemini adapter headless fix (`-p` + `--yolo`) | v1.0.0 |
| DX-020 | Key legend in dashboard footer | v1.0.0 |
| DX-021 | Orphan task cleanup on quit/restart | v1.0.0 |
| DX-022 | Dashboard stays open on completion | v1.0.0 |
| DX-023 | Interactive terminal panes (scrollable, focusable) | v1.0.0 |
| DX-025 | Monotonic task IDs (never overwrite completed) | v1.0.0 |
| DX-026 | Priority key handling (no lag under load) | v1.0.0 |
| DX-027 | User-spawnable shell panes | v1.0.0 |
| DX-028 | Git auto-commit per task (Tier 1) | v1.0.0 |
| DX-029 | Live agent streaming (stream-json + NDJSON parser) | v1.1.0 |
| DX-030 | Project name in dashboard header | v1.1.0 |
| DX-031 | Freeze completion timer on `all_complete` | v1.1.0 |

## Config Features (Already Shipped)

```bash
forge config claude.auth subscription    # Strip API keys (default)
forge config claude.auth api             # Pass API keys through
forge config claude.permissions yolo     # Full autonomy mode
forge config claude.permissions safe     # Read-only (default)
```

Same for codex.auth, codex.permissions, gemini.auth, gemini.permissions.
