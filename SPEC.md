# NXTG-Forge Orchestrator — Product Specification

**Version:** 0.1.0
**Author:** Asif W. (Founder, NXTG) + Claude (Opus 4.6)
**Date:** February 8, 2026
**Status:** APPROVED (synthesized from multi-AI brainstorming session)

---

## 1. Vision

NXTG-Forge Orchestrator is a **universal coordination engine for AI-powered development**. It lets developers harness Claude Code, Codex CLI, Gemini CLI, and future AI tools as a coordinated team — with governance, knowledge capture, and conflict prevention.

**One-sentence pitch:** "The tech lead that never sleeps — plans work, assigns it to the right AI tool, tracks progress, captures knowledge, and ensures quality."

## 2. Core Design Principles

1. **Forge Owns the Truth** — Agents emit events. Forge reconciles. No AI writes to another's config directly.
2. **Dual Engine** — Headless mode for deterministic CI/CD + TUI-aware mode for interactive Agent Teams.
3. **Linux Foundation Primitives** — Build on AGENTS.md, MCP, SKILL.md (AAIF-backed, vendor-neutral).
4. **File System = Persistence, MCP = Live Wire** — `.forge/` files are durable state. MCP server provides live queries.
5. **Knowledge Flywheel** — Every interaction generates intelligence. Forge captures and routes it automatically.
6. **Governance Never Sleeps** — Continuous validation of vision alignment, doc coverage, architecture compliance.

## 3. Architecture

### Three-Layer Stack

| Layer | Name | Repo | Purpose |
|-------|------|------|---------|
| 3 | Visual | `nxtg-ai/forge` (v3) | React dashboard, Governance HUD, Infinity Terminal |
| 2 | Intelligence | `nxtg-ai/forge-plugin` | Commands, agents, skills, hooks for each AI tool |
| 1 | Coordination | `nxtg-ai/forge-orchestrator` (THIS REPO) | Headless engine, MCP server, adapters, state management |

### Orchestrator Components

```
forge-orchestrator (Rust binary)
├── CLI Interface (clap)
│   ├── forge init          — scaffold .forge/ + detect tools
│   ├── forge plan          — create/show master plan
│   ├── forge run           — execute task on specific AI tool
│   ├── forge status        — show orchestration state
│   ├── forge sync          — force reconciliation
│   └── forge config        — set brain, preferences
│
├── Core Engine
│   ├── State Manager       — .forge/state.json read/write/lock
│   ├── Task Manager        — .forge/tasks/ CRUD + dependencies
│   ├── Event Logger        — .forge/events.jsonl append-only
│   ├── Knowledge Router    — classify + route to .forge/knowledge/
│   └── Governance Checker  — drift detection, quality gates
│
├── ForgeBrain Trait (pluggable LLM)
│   ├── ClaudeOpusBrain     — Claude API for reasoning
│   ├── GeminiBrain         — Gemini API for reasoning
│   ├── LocalBrain          — Fine-tuned local model (future)
│   └── RuleBasedBrain      — Heuristics only, no LLM (free tier)
│
├── Adapters (tool-specific renderers)
│   ├── Claude Adapter      — writes CLAUDE.md, .claude/
│   ├── Codex Adapter       — writes AGENTS.md, .agents/
│   ├── Gemini Adapter      — writes GEMINI.md, .gemini/
│   └── Generic Adapter     — future AI tools
│
├── MCP Server (forge-mcp, unified)
│   ├── forge_get_plan      — read master plan
│   ├── forge_get_tasks     — list tasks with status
│   ├── forge_claim_task    — lock task for agent
│   ├── forge_complete_task — mark done, trigger reconciliation
│   ├── forge_get_state     — full orchestration state
│   ├── forge_capture_knowledge — route learning to knowledge store
│   ├── forge_get_health    — project health score
│   ├── forge_check_drift   — compare work against vision
│   └── forge_get_knowledge — query knowledge base
│
└── Git Integration
    ├── Worktree Manager    — create/remove worktrees per task
    ├── Branch Convention   — forge/T-001-description naming
    └── Merge Strategy      — fast-forward or PR, configurable
```

### Shared State Directory (.forge/)

```
.forge/
├── plan.md              # Human-readable master plan
├── plan.yaml            # Machine-parseable plan (tasks, deps, estimates)
├── state.json           # Live orchestration state (who, what, locks)
├── events.jsonl         # Append-only event log
├── config.json          # Orchestrator config (tools, brain, preferences)
├── tasks/
│   ├── T-001.md         # Task contract (description, acceptance criteria)
│   ├── T-002.md
│   └── ...
├── knowledge/
│   ├── decisions/       # Architectural decisions (ADRs)
│   ├── learnings/       # Lessons learned
│   ├── research/        # Research findings
│   └── patterns/        # Discovered patterns
└── worktrees/           # Git worktree checkouts (gitignored)
```

## 4. User Flows

### Flow A: CEO Mode (SPEC.md detected)
```
$ forge init
Scanning project...
  Found: SPEC.md, CLAUDE.md, package.json, 127 source files
  Detected: Claude Code, Codex CLI

Auto-generating plan from SPEC.md...
  14 tasks across 2 AI tools. Approve? [Y/n] y

Plan saved to .forge/plan.md
Run `forge status` to see the board.
Run `forge run --all` to start execution.
```

### Flow B: CEO Mode (greenfield)
```
$ forge init
No SPEC.md found. Starting Vision Questionnaire...
  1. What are you building? > A REST API for task management
  2. What's the tech stack? > Rust + Axum + PostgreSQL
  3. What AI tools do you have? > [auto-detected: Claude, Codex]
  4. Quality priorities? > Tests first, then docs

Generating SPEC.md...
Generating plan (8 tasks)...
  Approve? [Y/n] y
```

### Flow C: Rescue Mode
```
$ forge init
Scanning project...
  12 failing tests, 3 type errors, 0 docs

Rescue plan generated:
  Phase 1: Fix 3 type errors (assigned: Claude)
  Phase 2: Fix 12 failing tests (assigned: Codex)
  Phase 3: Generate docs (assigned: Gemini)
  Approve? [Y/n] y
```

### Flow D: Headless Execution
```
$ forge run --task T-001 --agent claude
Executing T-001 via claude -p...
  Output: .forge/results/T-001.json
  Status: PASS (all acceptance criteria met)
  State updated. Next task: T-002 (unblocked).
```

## 5. Technology Choices

| Component | Technology | Rationale |
|-----------|-----------|-----------|
| Language | Rust | Single binary distribution, performance, credibility |
| Async runtime | Tokio | Industry standard for async Rust |
| CLI framework | Clap | Derive macros, great UX, completions |
| MCP server | mcp-rust-sdk or custom | AAIF-standard protocol |
| Serialization | Serde + serde_json/serde_yaml | Rust standard |
| Git integration | git2 (libgit2 bindings) | Worktree management, branch ops |
| HTTP client | Reqwest | For LLM API calls (ForgeBrain) |
| Template engine | Tera or Askama | For adapter markdown generation |

## 6. Implementation Phases

### Phase 1: Proof of Loop (Weeks 1-3)
**Goal:** Prove the orchestration loop works end-to-end.

Deliverables:
- [x] `forge init` — scaffold .forge/, detect installed AI tools
- [x] `forge plan` — read SPEC.md, create plan.md + tasks/*.md
- [x] `forge run --task T-001 --agent claude` — headless execution via `claude -p`
- [x] `forge status` — ASCII dashboard showing task state
- [x] State management (state.json read/write)
- [x] Claude adapter (write CLAUDE.md from .forge/ state)
- [x] Event logging (events.jsonl)

Success criteria: A developer can plan work, assign it to Claude, see the result, and verify state updates.

### Phase 2: MCP Live Wire (Weeks 4-6)
**Goal:** AI tools query live state via MCP.

Deliverables:
- [x] Unified forge-mcp server (stdio transport)
- [x] forge_get_tasks, forge_claim_task, forge_complete_task
- [x] forge_get_state, forge_get_plan
- [x] Claude Code connects as MCP client (verified via .mcp.json config)
- [x] Codex adapter + AGENTS.md generation
- [x] Gemini adapter + GEMINI.md generation

### Phase 3: Knowledge Flywheel (Weeks 7-8)
**Goal:** Automatic knowledge capture and routing.

Deliverables:
- [x] forge_capture_knowledge MCP tool
- [x] Knowledge classification (research, decision, learning, pattern)
- [x] Auto-generation of tool-specific SKILL.md from knowledge
- [x] forge_get_knowledge (query knowledge base)

### Phase 4: Governance Integration (Weeks 9-10)
**Goal:** Continuous governance validation.

Deliverables:
- [ ] Vision alignment checks on task completion
- [ ] Documentation sync detection
- [ ] Architecture compliance rules
- [ ] Drift detection (periodic)
- [ ] forge_check_drift MCP tool

### Phase 5: ForgeBrain Abstraction (Weeks 11-12)
**Goal:** Pluggable LLM brain.

Deliverables:
- [ ] ForgeBrain trait definition
- [ ] ClaudeOpusBrain implementation (Claude API)
- [ ] RuleBasedBrain implementation (no LLM, free tier)
- [ ] `forge config set brain <provider>` command
- [ ] Brain-powered plan decomposition from SPEC.md

## 7. Non-Goals (Not in v0.1)

- Web dashboard integration (Phase 5 in overall roadmap, not this repo)
- Fine-tuned local model (requires training data from real usage)
- Cloud sync / multi-developer orchestration (commercial feature)
- Codex App Server protocol integration (complex, not needed for MVP)
- Windows native support (WSL2 is fine for now)

## 8. Success Metrics

| Metric | Target |
|--------|--------|
| `forge init` to first task complete | < 5 minutes |
| Headless task execution overhead | < 2 seconds |
| State.json read/write latency | < 10ms |
| Binary size | < 20MB |
| Zero external runtime dependencies | Yes (single binary) |

## 9. Competitive Positioning

| Product | What It Does | What Forge Does Better |
|---------|-------------|----------------------|
| Claude Squad | Manages Claude terminals | Orchestrates ALL AI tools, not just Claude |
| Claude-Flow | Multi-agent Claude orchestration | Vendor-neutral, not Claude-locked |
| Gas Town | Task management for Claude | Knowledge flywheel, governance, multi-AI |
| Codex CLI | Headless code execution | Coordinates Codex WITH other tools |
| CCManager | Session management | Task-level orchestration, not just sessions |

## 10. Revenue Path

1. **Now:** MIT open source (build community)
2. **12 months:** Commercial features (cloud sync, team orchestration, advanced governance)
3. **18 months:** Linux Foundation donation (standardize .forge/ format, become AAIF project)

---

*This spec was synthesized from a multi-AI brainstorming session involving Claude Opus 4.6, ChatGPT 5.2, Gemini 3 Pro, and Asif (Founder). See: v3/.asif/notes.md for the full conversation.*
