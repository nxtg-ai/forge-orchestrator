# Forge Orchestrator

Universal coordination engine for AI-powered development. Plans work, assigns it to the right AI tool (Claude Code, Codex CLI, Gemini CLI), tracks progress, and prevents conflicts.

## Quick Start

```bash
# Install
cargo install --path .

# Initialize in any project
cd your-project
forge init --name "My Project"

# Generate a plan from your spec
forge plan --generate

# Check status
forge status

# Run a task headlessly
forge run --task T-001 --agent claude

# Sync state and render adapter configs
forge sync
```

## What It Does

Forge is the **tech lead that never sleeps**. It coordinates multiple AI coding tools working on the same codebase:

1. **`forge init`** — Scans your project, detects installed AI tools (Claude, Codex, Gemini), scaffolds `.forge/` state directory
2. **`forge plan --generate`** — Reads your SPEC.md, decomposes it into tasks, assigns each to the best AI tool
3. **`forge run`** — Executes a task headlessly via the assigned AI tool's CLI
4. **`forge status`** — ASCII dashboard showing task board, progress, file locks, recent events
5. **`forge sync`** — Reconciles state and renders tool-specific config files (CLAUDE.md, AGENTS.md, GEMINI.md)
6. **`forge mcp`** — Starts the MCP server (stdio transport) so AI tools can query and update state in real-time

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│  CLI (clap)                                             │
│  forge init | plan | run | status | sync | mcp          │
├─────────────────────────────────────────────────────────┤
│  Brain (pluggable)                                      │
│  RuleBasedBrain | OpenAIBrain | ClaudeBrain (future)    │
├─────────────────────────────────────────────────────────┤
│  Core Engine                                            │
│  TaskManager | StateManager | EventLogger | PlanManager │
├─────────────────────────────────────────────────────────┤
│  Adapters                                               │
│  ClaudeAdapter | CodexAdapter | GeminiAdapter           │
├─────────────────────────────────────────────────────────┤
│  .forge/ (file-based state)                             │
│  state.json | tasks/ | events.jsonl | plan.md           │
└─────────────────────────────────────────────────────────┘
```

### Dual Engine Design

- **Brain 1 (Deterministic):** State management, file locking, event logging, git ops — all in Rust, zero LLM tokens
- **Brain 2 (ForgeBrain trait):** Plan decomposition, task assignment, drift detection — pluggable LLM backends

### ForgeBrain Implementations

| Brain | Cost | Use Case |
|-------|------|----------|
| `RuleBasedBrain` | Free | Keyword heuristics, no API calls |
| `OpenAIBrain` | Paid | GPT-4o/o3/o4-mini for intelligent decisions |
| `ClaudeBrain` | Future | Claude API integration |
| `LocalBrain` | Future | Fine-tuned local model |

## `.forge/` Directory Structure

```
.forge/
├── state.json          # Project state, tool inventory, file locks
├── plan.md             # Master plan (human-readable)
├── events.jsonl        # Append-only event log
├── tasks/
│   ├── T-001.json      # Task definitions
│   └── T-001.md        # Human-readable task cards
├── results/            # Agent execution results
└── knowledge/          # Captured learnings
    ├── decisions/
    ├── learnings/
    ├── research/
    └── patterns/
```

## MCP Server

Forge includes a built-in MCP (Model Context Protocol) server that lets AI tools interact with orchestration state in real-time.

```bash
# Start the MCP server (stdio transport)
forge mcp --project /path/to/project
```

### Available MCP Tools

| Tool | Description |
|------|-------------|
| `forge_get_tasks` | List all tasks with status, assignments, dependencies |
| `forge_claim_task` | Claim a task for an agent — locks files, sets status |
| `forge_complete_task` | Mark task done — unlocks files, shows newly available tasks |
| `forge_get_state` | Full orchestration state (tools, summary, locks) |
| `forge_get_plan` | Read the master plan |

### Connecting Claude Code

Add to your project's `.mcp.json`:

```json
{
  "mcpServers": {
    "forge": {
      "command": "forge",
      "args": ["mcp", "--project", "."],
      "type": "stdio"
    }
  }
}
```

Then Claude Code can call `forge_get_tasks()`, `forge_claim_task()`, etc. directly.

## File Locking

Forge prevents multiple AI agents from editing the same file simultaneously. When an agent claims a task, its files are locked in `state.json`. Other agents see conflicts before starting work.

## Development

```bash
# Build
cargo build

# Test
cargo test

# Build optimized release (2-3 MB binary)
cargo build --release
```

## License

MIT
