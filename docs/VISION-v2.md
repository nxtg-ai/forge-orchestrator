# Forge Orchestrator — Vision v2.0: "Beyond CLI Spawning"

> Integrating RESEARCH-001 findings (66 sources, 6 arXiv papers) into a comprehensive roadmap.
> Date: 2026-02-15 | Status: APPROVED

---

## The Problem We Solved (v1.x)

Forge-orchestrator proved that multi-agent AI orchestration works: spawn Claude, Codex, and Gemini CLI processes, coordinate them through a dependency DAG, and deliver results through a beautiful TUI dashboard. Voice-jib-jab shipped 34/34 tasks with zero failures. SynApps dogfood hit 27/39 (69%) before rate limits intervened.

**But v1.x has a fatal flaw: it's built on CLI process spawning.**

## The Problem We Must Solve (v2.0)

RESEARCH-001 (5,799 words, 66 sources) revealed three truths:

1. **Anthropic actively blocks subscription-credential CLI orchestration.** January 2026 crackdown — account bans, server-side client fingerprinting, telemetry validation. ToS Section D.4 explicitly prohibits "access through automated means except via API Key." Our subscription adapter is prohibited.

2. **Subscription tiers are operationally unreliable for orchestration.** Codex: 10-60 cloud tasks/5h. Gemini Pro: 25-100 requests/day. Claude: throttled + weekly caps. These ceilings make sustained multi-agent builds impossible on subscriptions.

3. **API-key SDK orchestration is explicitly blessed by ALL providers** and costs ~$149/mo for 100 tasks/day with smart routing — less than a single Claude Max 20x subscription.

**The safe, scalable path forward: evolve from CLI process spawning to direct API/SDK orchestration.**

---

## Architecture Evolution

```
v1.x (Current)                          v2.0 (Target)
─────────────                           ─────────────
forge-orca                              forge-orca
    │                                       │
    ├── spawn claude -p "task"              ├── reqwest → Anthropic Messages API
    ├── spawn codex exec "task"             ├── reqwest → OpenAI Chat API
    └── spawn gemini -p "task"              ├── reqwest → Gemini GenerateContent API
                                            │
    Process stdout capture                  ├── PTY bridge → claude (interactive)
    ↓                                       ├── PTY bridge → codex (interactive)
    Text parsing                            └── PTY bridge → gemini (interactive)
    ↓
    Task completion detection               Structured API responses
                                            ↓
                                            Native Rust parsing
                                            ↓
                                            Real-time streaming + tool orchestration
```

**Dual-mode architecture:**
- **API Mode (default):** Direct HTTP calls to provider APIs. Full control, proper rate limit headers, structured responses, cost tracking. For headless orchestration.
- **Stargate Mode (interactive):** PTY-bridged CLI sessions embedded in dashboard panes. For when the user wants to watch, interact, and steer agents in real-time.

---

## Roadmap

### v1.3.0 "The Safe Operator" — Immediate (1-2 days)

**Goal:** Codify research findings into the product. Make forge safe by default.

| DX | Item | Description |
|----|------|-------------|
| DX-037 | Quota Monitoring | Per-provider usage tracking in dashboard footer: `Claude: 12/50 tasks (5h) │ Codex: 8/60 tasks (5h) │ Gemini: 45/1000 RPD`. Parse reset timestamps from 429s. |
| DX-038 | Subscription Risk Warning | Anthropic subscription = blocked by default. Require `--i-accept-subscription-risk` flag. Show legal disclaimer from RESEARCH-001 Section 8.4. |
| DX-039 | API-First Default | Change default auth mode from `subscription` to `api`. `forge config claude.auth api` becomes the default. Subscription requires explicit opt-in. |
| DX-040 | Adaptive Pacing | Replace fixed 64-179s cooldowns with Adaptive Token Bucket (arXiv:2510.04516). Start conservative, increase when no 429s, halve on rate limit. 97% fewer 429 errors. |

### v2.0.0 "Stargate" — Medium-term (2-4 weeks)

**Goal:** Dual-mode architecture. API orchestration + embedded interactive TUIs.

| DX | Item | Description |
|----|------|-------------|
| DX-024 | PTY Bridge | `portable-pty` crate for PTY allocation. Each dashboard pane becomes a real terminal running the actual CLI. User can tab between agents and interact directly. Prerequisite: DX-023 + DX-027 (done). |
| DX-041 | Direct API Orchestration | `reqwest` → Anthropic Messages API, OpenAI Chat Completions, Gemini GenerateContent. Structured tool-use protocol per provider. No CLI process spawning. |
| DX-042 | Smart Task Router | DAAO-inspired (arXiv:2509.11079) difficulty-aware routing: simple→codex-mini ($0.045/task), medium→Sonnet ($0.105/task), complex→Opus ($0.175/task). Estimated 40-60% cost reduction vs uniform model assignment. |
| DX-043 | Cost Tracking | Per-task token counting, per-provider cost accumulation, budget alerts. Dashboard shows: `Session: $4.23 │ Today: $12.87 │ Month: $142.50`. Configurable monthly budget cap. |
| DX-044 | Tool Protocol | Translate forge's file operations into provider-native tool-use: Anthropic tool_use blocks, OpenAI function calling, Gemini function declarations. Each provider gets tool definitions matching their SDK capabilities. |
| DX-028 T2 | Git Worktrees | Each agent gets `git worktree` for parallel isolation. Commits independently, merges on completion. `forge config git.strategy single|worktree|branch`. |

### v3.0.0 "The Platform" — Long-term (2-3 months)

**Goal:** Forge becomes infrastructure, not just a CLI tool.

| DX | Item | Description |
|----|------|-------------|
| DX-045 | Forge as MCP Server | Expose forge orchestration as MCP tools: `forge_plan`, `forge_dispatch`, `forge_status`. Any MCP client (Claude Code, Codex, Gemini CLI) can consume forge as a tool. Forge orchestrating forge. |
| DX-046 | Multi-Project Orchestration | Orchestrate across multiple repos simultaneously. SynApps frontend + backend + docs in one forge session. Shared dependency graph across projects. |
| DX-047 | forge-ui Integration | Web dashboard (v3/forge-ui) as alternative to TUI. Real-time WebSocket streaming of agent panes. UAT panel. Cost dashboard. Configuration UI. |
| DX-048 | Plugin Ecosystem | User-defined adapters for any LLM CLI or API. `forge plugin install deepseek-adapter`. Community-contributed brain strategies. |
| DX-049 | ClaudeBrain v2 | AI-powered task routing using lightweight classifier. Learn from past task outcomes which provider+model combo works best for each task type. Reinforcement learning loop. |

---

## Research Integration: Key Decisions

### Decision 1: Anthropic Subscription Mode — PERMANENTLY DISABLED

**Source:** RESEARCH-001 Sections 1.1, 4.1, 7.1
**Risk Score:** CRITICAL (95% detection, 90% enforcement, catastrophic impact)
**Action:**
- `forge config claude.auth subscription` → prints warning + requires `--i-accept-subscription-risk`
- Default: `claude.auth api`
- Dashboard shows red warning banner when subscription mode active
- Legal disclaimer from RESEARCH-001 Section 8.4

### Decision 2: OpenAI Subscription — CONDITIONAL, PACED

**Source:** RESEARCH-001 Sections 1.2, 2.2, 7.1
**Risk Score:** MODERATE (40% detection, 15% enforcement, graduated response)
**Action:**
- `codex exec` is officially supported — subscription mode allowed
- Strict pacing: max 60% of 5h quota (6-36 cloud tasks/5h for Team)
- Adaptive Token Bucket pacing (not fixed delays)
- Graceful degradation: pause + offer API-key fallback on `usage_limit_reached`

### Decision 3: Google Subscription — GO WITH PACING

**Source:** RESEARCH-001 Sections 1.3, 2.3, 7.1
**Risk Score:** LOW (10% detection, 5% enforcement, graduated response)
**Action:**
- Apache 2.0 CLI, no ToS prohibition, fully open
- Pacing: respect Pro sub-quota (~25-100/day), auto-fallback to Flash
- Monitor RPM (60 for Google Account auth)

### Decision 4: API-Key Mode — PRIMARY PATH

**Source:** RESEARCH-001 Sections 6, 7.2, 8.3
**Risk Score:** SAFE across all providers
**Action:**
- Default for all providers
- Smart routing reduces cost to ~$149/mo for 100 tasks/day
- Proper rate-limit headers enable precise backpressure
- No account risk, no bans, no throttling beyond documented limits

### Decision 5: Adaptive Pacing > Fixed Delays

**Source:** arXiv:2510.04516 (Adaptive Token Bucket)
**Evidence:** 97% reduction in 429 errors with only 19% duration increase
**Action:**
- Replace DX-033 fixed 64-179s delays with ATB algorithm
- Start conservative (1 task/3min for subscription, no delay for API)
- Increase rate when no 429s detected in sliding window
- Immediately halve rate on any rate limit signal
- Never exceed 60% of documented quota

### Decision 6: Difficulty-Aware Task Routing

**Source:** arXiv:2509.11079 (DAAO)
**Evidence:** Cost-performance-aware routing produces equivalent quality at lower cost
**Action (v2.0):**
- Classify tasks by estimated complexity (lines of code, file count, description keywords)
- Simple → codex-mini ($0.045/task) or Gemini Flash (free)
- Medium → Claude Sonnet ($0.105) or GPT-4.1 ($0.060)
- Complex → Claude Opus ($0.175) or GPT-5.2-codex ($0.525)
- User configurable: `forge config routing.strategy smart|round-robin|single`

---

## Cost Model: Subscription vs API

From RESEARCH-001 Section 3.4:

```
Monthly cost at 100 tasks/day (30 days):

Subscription approach:
  Claude Max 20x:  $200/mo  ← BANNED for orchestration
  ChatGPT Pro:     $200/mo  ← 10-60 tasks/5h cap
  Gemini Pro:       $20/mo  ← 25-100 Pro req/day cap
  TOTAL:           $420/mo  ← Unreliable, ban risk

API approach (smart routing):
  30% Gemini free (simple):     $0/mo
  40% codex-mini (medium):     $54/mo
  30% Claude Sonnet (complex): $95/mo
  TOTAL:                      $149/mo  ← Reliable, no risk

Savings: $271/mo (65% reduction) + zero ban risk
```

---

## Stargate Architecture (DX-024)

The crown jewel of v2.0. Three portals into three AI universes, one command center.

```
┌─ Forge Dashboard ───────────────────────────────────────────┐
│ PROJECT: synapps │ PHASE: BUILD │ 12/24 tasks │ $4.23      │
├─────────────────────────────────────────────────────────────┤
│ ┌─ Claude (PTY) ──────────┐ ┌─ Codex (PTY) ──────────────┐ │
│ │ > Implementing auth...  │ │ $ codex exec --json        │ │
│ │ Reading src/auth/...    │ │ Running task T-003...       │ │
│ │ Editing login.ts...     │ │ Modified: api/routes.ts    │ │
│ │ [Claude's actual TUI]   │ │ [Codex's actual TUI]       │ │
│ │                         │ │                            │ │
│ │ ▌User can type here     │ │ ▌User can type here        │ │
│ └─────────────────────────┘ └────────────────────────────┘ │
│ ┌─ Gemini (PTY) ──────────┐ ┌─ Summary ─────────────────┐ │
│ │ > Writing tests...      │ │ Completed: T-001, T-002    │ │
│ │ gemini-2.5-pro          │ │ In Progress: T-003, T-004  │ │
│ │ [Gemini's actual TUI]   │ │ Pending: T-005..T-024      │ │
│ │                         │ │ Rate Limits: All OK        │ │
│ │ ▌User can type here     │ │ Cost: $4.23 session        │ │
│ └─────────────────────────┘ └────────────────────────────┘ │
│ Claude: 3/50 (5h) │ Codex: 8/60 (5h) │ Gemini: 45/1000    │
└─────────────────────────────────────────────────────────────┘
```

**Technical approach:**
1. `portable-pty` crate for PTY allocation (cross-platform)
2. Each agent pane gets its own PTY session
3. PTY output piped to ratatui widget via `tokio::io::AsyncBufReadExt`
4. Keystroke forwarding: focused pane receives keyboard input
5. ANSI escape sequence passthrough for full TUI rendering
6. Session persistence: PTY sessions survive dashboard restart

**Stargate enables both modes simultaneously:**
- Headless API tasks run in background (no PTY needed)
- Interactive tasks get full PTY panes for live observation
- User can spawn ad-hoc PTY sessions (already works via DX-027)
- Hybrid: API dispatches task, then user "attaches" to watch progress

---

## Implementation Priority

```
IMMEDIATE (v1.3.0)     MEDIUM-TERM (v2.0)       LONG-TERM (v3.0)
────────────────────    ──────────────────────    ─────────────────
DX-037 Quota Monitor    DX-024 PTY Bridge        DX-045 MCP Server
DX-038 Sub Risk Warn    DX-041 API Orchestrate   DX-046 Multi-Project
DX-039 API-First Def    DX-042 Smart Router      DX-047 forge-ui
DX-040 Adaptive Pace    DX-043 Cost Tracking     DX-048 Plugin System
                        DX-044 Tool Protocol     DX-049 ClaudeBrain v2
                        DX-028 Git Worktrees
```

**Why this order:**
1. v1.3.0 makes forge SAFE (no ban risk, informed users)
2. v2.0 makes forge POWERFUL (direct API, interactive TUIs, cost optimization)
3. v3.0 makes forge a PLATFORM (infrastructure for the ecosystem)

---

## Competitive Landscape

From RESEARCH-001 Section 5.2:

| Project | Stars | Approach | Rate Limit Strategy |
|---------|-------|----------|---------------------|
| AWS CLI Agent Orchestrator | ~100 | tmux + MCP | Not documented |
| claude-octopus | ~50 | Multi-CLI spawn | Not documented |
| claude-flow | ~200 | Claude Code spawn | Not documented |
| **forge-orchestrator** | **—** | **CLI spawn → API+PTY** | **ATB pacing, quota monitoring, risk warnings, provider rotation** |

**Our moat:** No other orchestrator has:
- Adaptive rate limit management (arXiv-backed ATB algorithm)
- Provider-specific risk assessment and safety features
- Quota monitoring with real-time dashboard visualization
- Dual-mode architecture (API for headless, PTY for interactive)
- Smart difficulty-aware task routing

---

## References

- RESEARCH-001: `docs/research/cli-subscription-gating-analysis.md` (66 sources)
- arXiv:2510.04516 — Adaptive Token Bucket (ATB) rate limiting
- arXiv:2511.03279 — RL-based multi-objective rate limiting
- arXiv:2411.15997 — FairServe (provider-side detection patterns)
- arXiv:2509.11079 — DAAO (difficulty-aware agent orchestration)
- arXiv:2511.15755 — Multi-agent orchestration validation (80x improvement)
- arXiv:2312.00989 — Scrappy (cryptographic rate limiting — future direction)
