# Forge Orchestrator — DX Backlog

> From live dogfood sessions (2026-02-10/11) + RESEARCH-001 findings (2026-02-15).
> Vision: `docs/VISION-v2.md` | Research: `docs/research/cli-subscription-gating-analysis.md`

## Open Items — v1.3.0 "The Safe Operator" (4 items)

### DX-037: Quota Monitoring — Provider Usage in Status/Dashboard
- **Priority:** HIGH (v1.3.0)
- **Where:** `src/tui/app.rs` + `src/tui/ui.rs` + `src/cli/status.rs`
- **Problem:** No visibility into how much quota each provider has consumed or when rate limits will reset. User discovers limits only when tasks fail.
- **Solution:** Track tasks dispatched per provider per 5-hour window. Show in dashboard footer: `Claude: 3/50 (5h) │ Codex: 8/60 (5h) │ Gemini: 45/1000 RPD`. Parse reset timestamps from 429 responses to show countdown.
- **Source:** RESEARCH-001 Section 2 (rate limit architecture per provider)

### DX-038: Subscription Risk Warning
- **Priority:** CRITICAL (v1.3.0)
- **Where:** `src/cli/config.rs` + `src/tui/app.rs` + `src/cli/start.rs`
- **Problem:** Anthropic ACTIVELY BLOCKS subscription-credential orchestration (Jan 2026 crackdown). Users risk account bans.
- **Solution:** Anthropic subscription mode = blocked by default. Require `--i-accept-subscription-risk` flag. Show legal disclaimer. Dashboard red warning banner when any subscription adapter active.
- **Source:** RESEARCH-001 Sections 1.1, 4.1, 7.1, 8.4

### DX-039: API-First Default Auth Mode
- **Priority:** HIGH (v1.3.0)
- **Where:** `src/core/state.rs` + `src/cli/config.rs`
- **Problem:** Default auth mode is `subscription`, which is the risky path. Should default to `api`.
- **Solution:** Change default from `subscription` to `api` for all providers. `forge init` prompts for API keys. `forge config <provider>.auth subscription` requires explicit opt-in.
- **Source:** RESEARCH-001 Section 8.3 (API as primary path)

### DX-040: Adaptive Token Bucket Pacing
- **Priority:** MEDIUM (v1.3.0)
- **Where:** `src/tui/app.rs` + `src/cli/start.rs`
- **Problem:** Fixed 64-179s delays (DX-033) are inefficient — too slow when no rate limits, too fast when approaching limits.
- **Solution:** Replace with Adaptive Token Bucket (ATB) algorithm. Start conservative, increase rate when no 429s detected in sliding window, immediately halve on rate limit signal. Never exceed 60% of documented quota. 97% fewer 429 errors per arXiv paper.
- **Source:** arXiv:2510.04516 (Adaptive Rate Limiting)

## Open Items — v2.0.0 "Stargate" (6 items)

### DX-024: Forge Stargate — Embedded Interactive Agent TUIs
- **Priority:** VISION (v2.0)
- **Where:** New architecture — PTY bridge per agent pane
- **Problem:** Current agent panes show captured stdout text. The dream is embedding ACTUAL running TUIs (Claude Code's TUI, Gemini's TUI, Codex's TUI) inside forge dashboard panes.
- **Solution:** `portable-pty` crate for PTY allocation. Each pane becomes a real terminal. Keystroke forwarding when focused. ANSI passthrough for full TUI rendering.
- **Prerequisite:** DX-023 (done) and DX-027 (done) — scrollable/focusable panes + shell panes prove the pattern.

### DX-041: Direct API Orchestration
- **Priority:** HIGH (v2.0)
- **Where:** New `src/adapters/api/` module
- **Problem:** CLI spawning is fragile, unstructured, and subject to provider gating. Direct API calls are blessed, structured, and rate-limit-aware.
- **Solution:** `reqwest` → Anthropic Messages API, OpenAI Chat Completions, Gemini GenerateContent. Structured tool-use per provider. Streaming responses. Proper rate-limit header parsing.
- **Source:** RESEARCH-001 Sections 6.1, 6.4

### DX-042: Smart Task Router (DAAO-inspired)
- **Priority:** MEDIUM (v2.0)
- **Where:** New `src/brain/router.rs`
- **Problem:** All tasks dispatched to same model regardless of complexity. Wastes money on simple tasks, under-powers complex ones.
- **Solution:** Classify task complexity (LOC estimate, file count, keywords). Simple → codex-mini ($0.045), Medium → Sonnet ($0.105), Complex → Opus ($0.175). 40-60% cost reduction vs uniform assignment.
- **Source:** arXiv:2509.11079 (DAAO)

### DX-043: Cost Tracking & Budget Management
- **Priority:** MEDIUM (v2.0)
- **Where:** `src/core/cost.rs` + `src/tui/ui.rs`
- **Problem:** No visibility into API costs. Users can accidentally spend hundreds.
- **Solution:** Per-task token counting, per-provider cost accumulation, budget alerts. Dashboard: `Session: $4.23 │ Today: $12.87 │ Month: $142.50`. Configurable monthly cap.

### DX-044: Native Tool Protocol
- **Priority:** HIGH (v2.0)
- **Where:** `src/adapters/api/tools.rs`
- **Problem:** CLI adapters rely on text output parsing. API mode needs structured tool-use.
- **Solution:** Translate file operations into provider-native tool definitions: Anthropic tool_use blocks, OpenAI function calling, Gemini function declarations.

### DX-028 Tier 2/3: Git Worktrees for Parallel Isolation
- **Priority:** FUTURE (v2.0)
- **Where:** `src/tui/app.rs` + new `src/core/git.rs`
- **Problem:** Tier 1 auto-commit commits to current branch. Parallel agents editing same files = conflicts.
- **Solution:** Each agent gets `git worktree`, commits independently, merges on completion. `forge config git.strategy single|worktree|branch`.

## Open Items — v3.0.0 "The Platform" (5 items)

### DX-045: Forge as MCP Server
- **Priority:** VISION (v3.0)
- **Problem:** Forge is a consumer of AI, not a provider. As MCP server, any MCP client could orchestrate via forge.
- **Solution:** Expose `forge_plan`, `forge_dispatch`, `forge_status` as MCP tools. Forge orchestrating forge.

### DX-046: Multi-Project Orchestration
- **Priority:** FUTURE (v3.0)
- **Problem:** One forge session = one project. Real work spans repos.
- **Solution:** Shared dependency graph across multiple project roots.

### DX-047: forge-ui Web Dashboard Integration
- **Priority:** FUTURE (v3.0)
- **Problem:** TUI is powerful but limited to terminal users. Web dashboard reaches everyone.
- **Solution:** Real-time WebSocket streaming to v3/forge-ui. UAT panel. Cost dashboard. Configuration UI.

### DX-048: Plugin Ecosystem
- **Priority:** FUTURE (v3.0)
- **Problem:** Only 3 hardcoded adapters. Users want DeepSeek, Mistral, local models.
- **Solution:** User-defined adapters. `forge plugin install deepseek-adapter`. Community brain strategies.

### DX-049: ClaudeBrain v2
- **Priority:** FUTURE (v3.0)
- **Problem:** Rule-based task routing is static. Should learn from outcomes.
- **Solution:** Lightweight classifier trained on past task outcomes. RL loop for provider+model selection.

### DX-051: Three-Tier Validation (T→V→U Pipeline)
- **Priority:** HIGH (v1.4.0)
- **Where:** `src/tui/uat_app.rs` + `src/tui/uat_ui.rs` + `src/core/task.rs` + plan generation
- **Problem:** Current UAT board shows ALL tasks (T+V) with no hierarchy. Human UAT is conflated with AI verification. The AI can't validate UX/DX alignment with product vision — only a human can.
- **Solution:** Three-tier validation pipeline:
  - **T-xxx** (Build): AI implements the feature
  - **V-xxx** (Verify): AI validates technically (tests, code quality)
  - **U-xxx** (UAT): Human validates UX/DX (feel, flow, vision alignment)
  - Plan phase auto-generates U-xxx tasks for UI/UX/DX-facing outcomes with human-testable acceptance criteria ("Can user navigate login→dashboard in <3 clicks?")
  - UAT board filters to U-xxx by default, shows T→V→U hierarchy with AI validation results inline
  - Detail panel shows: U-task criteria, V-task AI validation summary + which AI ran it, T-task build context
- **DX Impact:** Makes `forge uat` the human-in-the-loop checkpoint in the agentic dev pipeline. AI handles T+V autonomously, human focuses only on what AI literally can't judge.
- **Source:** Dogfood Session 24 (2026-02-18)

### DX-052: UAT Board Improvements (Shipped Fixes)
- **Priority:** SHIPPED (v1.2.0+uat)
- **What shipped:** Persistent pass/fail status (.forge/uat-status.json), scrollable task list (ListState), description fallback when no acceptance criteria, `u` key unmark, report generation (.forge/uat-report.md), tighter related-task matching (stop words + 5-char min + cap at 5)
- **Source:** Dogfood Session 24 (2026-02-18)

## Completed Items (38 of 53)

| DX | Description | Version |
|----|-------------|---------|
| DX-050 | Builder Mode (TUI dispatch, completion detection, idle TUI spawn) | v1.2.0 |
| DX-052 | UAT Board Improvements (persist, scroll, report, unmark) | v1.2.0 |

| DX | Description | Version |
|----|-------------|---------|
| DX-001-008 | Init, plan, config, status fixes | v0.2.0 |
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
| DX-032 | Standalone UAT TUI (task selector, criteria view, finding capture) | v1.2.0 |
| DX-033 | Subscription pacing (64-179s random delays for subscription auth) | v1.2.1 |
| DX-034 | Rate limit detection (provider-specific 429 parsing) | v1.2.1 |
| DX-035 | Exponential backoff on rate limit (60s→600s, provider pause) | v1.2.1 |
| DX-036 | Provider rotation (optional task redistribution) | v1.2.1 |

## Config Features (Already Shipped)

```bash
forge config claude.auth subscription    # Subscription mode (risky — see DX-038)
forge config claude.auth api             # API key mode (recommended — see DX-039)
forge config claude.permissions yolo     # Full autonomy mode
forge config claude.permissions safe     # Read-only (default)
forge config scheduler.rotation enabled  # Enable provider rotation on rate limit
forge config scheduler.pacing 64-179     # Subscription pacing delay range (seconds)
```

Same for codex.auth, codex.permissions, gemini.auth, gemini.permissions.
