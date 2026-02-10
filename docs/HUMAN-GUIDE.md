# Forge Orchestrator — Human Walkthrough

**From zero to AI-orchestrated development in 10 minutes.**

This guide walks you through using Forge to build a real project using Claude Code, Codex CLI, and Gemini CLI as a coordinated team.

---

## Prerequisites

You need at least ONE of these AI CLIs installed:

| Tool | Install | Auth |
|:-----|:--------|:-----|
| [Claude Code](https://docs.anthropic.com/claude-code) | `npm install -g @anthropic-ai/claude-code` | Claude Max subscription (covers both interactive and headless) |
| [Codex CLI](https://github.com/openai/codex) | `npm install -g @openai/codex` | `codex login` (ChatGPT Pro subscription) |
| [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `npm install -g @anthropic-ai/gemini-cli` | `gemini login` (Google AI Pro subscription) |

You also need:
- A project that is a **git repository** (`git init` if not already)
- (Optional) An `OPENAI_API_KEY` for AI-powered plan generation

## Step 1: Install Forge

```bash
curl -sSL https://raw.githubusercontent.com/nxtg-ai/forge-orchestrator/main/install.sh | sh
```

Verify: `forge --version` should print `forge-orchestrator 0.1.x`

## Step 2: Initialize Your Project

```bash
cd your-project
forge init
```

This creates a `.forge/` directory with state tracking. Forge auto-detects which AI CLIs you have installed.

**What you'll see:**

```
✓ Initialized Forge for "your-project"
  Detected tools: claude, codex, gemini
  Brain: rule_based (default)
```

## Step 3: Write Your Spec

Create a `SPEC.md` file in your project root. This is your vision document — what you want to build.

```markdown
# My Project

## Vision
A REST API for managing user accounts.

## Requirements
- User registration with email/password
- JWT authentication
- Profile CRUD endpoints
- Input validation
- Unit tests for all endpoints
```

**The more detailed your spec, the better the plan.**

## Step 4: Configure the Brain (Optional but Recommended)

The default brain uses keyword heuristics (free, offline). For better plans:

```bash
# Set your OpenAI API key
echo "OPENAI_API_KEY=sk-..." >> .env

# Switch to the OpenAI brain
forge config brain openai
forge config brain.model gpt-4.1    # fast and cheap
```

## Step 5: Generate the Plan

```bash
forge plan --generate
```

This reads your SPEC.md and decomposes it into tasks with:
- Dependencies (which tasks must finish before others can start)
- Agent assignments (architecture → claude, implementation → codex, docs → gemini)
- File locks (which files each task will touch)
- Acceptance criteria

**What you'll see:**

```
✓ Brain generated 15 tasks from SPEC.md

Task Board:
  T-001: Project scaffolding          [codex]   deps: none
  T-002: Define data models           [claude]  deps: T-001
  T-003: Implement user registration  [codex]   deps: T-002
  ...
```

Review the plan: `forge status` shows the full task board.

## Step 6: Authenticate Your AI Tools

Each CLI needs to be logged in before Forge can use it:

```bash
# Claude Code — already authenticated if you're reading this in Claude Code
# Otherwise: needs API credits at console.anthropic.com for headless mode

# Codex CLI — opens browser for OAuth
codex login

# Gemini CLI — opens browser for OAuth
gemini login
```

**Important:** Do NOT set `ANTHROPIC_API_KEY` in your environment — it overrides subscription auth and routes to API billing. All three CLIs use their respective subscriptions for headless execution.

## Step 7: Run Everything

### Option A: Full Autonomous Mode (Recommended)

```bash
forge start
```

This launches ALL detected agents in parallel:
- Each agent gets its own thread
- Tasks are auto-claimed based on assignment
- Dependencies are respected (blocked tasks wait)
- Completed tasks unlock the next batch
- Failed tasks are logged and skipped

**What you'll see:**

```
FORGE — Autonomous Orchestration
========================================

  → agents: claude, codex, gemini
  → tasks:  15 total, 0 done, 15 remaining

  [codex] → Executing T-001: Project scaffolding
  [claude] 3 tasks blocked, waiting for dependencies...
  [gemini] 2 tasks blocked, waiting for dependencies...
  [codex] ✓ T-001 completed (45s)
  [codex] → Executing T-003: Implement user registration
  [claude] → Executing T-002: Define data models
  [gemini] 2 tasks blocked, waiting for dependencies...
  ...
```

### Option B: Single Agent

```bash
forge start --agent codex    # Only run codex tasks
```

### Option C: CEO Mode (Fully Autonomous)

```bash
forge start --ceo
```

CEO Mode loops `forge start` until every task is complete (up to 5 passes). Between passes it:
- Resets failed tasks back to `pending` for automatic retry
- Waits 30 seconds before the next pass
- Stops when all tasks are `completed` or the pass limit is hit

This is the "walk away and come back to a finished project" mode.

### Option D: One Task at a Time

```bash
forge run --task T-001 --agent codex
```

## Step 8: Monitor Progress

While `forge start` is running (or after):

```bash
forge status
```

Shows completion percentage, which tasks succeeded/failed, and agent activity.

## Step 9: Review Results

Failed task output is saved to `.forge/results/T-XXX.txt`. Read these to understand what went wrong:

```bash
cat .forge/results/T-003.txt
```

To re-run a failed task, reset its status and run again:

```bash
# Edit .forge/tasks/T-003.json — change "status": "failed" to "status": "pending"
forge run --task T-003 --agent codex
```

## Step 10: Sync and Continue

After tasks complete, sync renders updated config files for each AI tool:

```bash
forge sync
```

This regenerates `CLAUDE.md`, `AGENTS.md`, and `GEMINI.md` with current state, so each tool has fresh context if you continue working interactively.

---

## Common Issues

### "Credit balance is too low" (Claude)

This usually means `ANTHROPIC_API_KEY` is set in your environment, which routes requests to API billing instead of your Max subscription. Fix:

```bash
unset ANTHROPIC_API_KEY   # Remove the API key from your shell
# Also check .env files for ANTHROPIC_API_KEY and remove it
```

If you genuinely hit your subscription usage limit, wait for it to reset or upgrade your plan.

### "Not inside a trusted directory" (Codex)

Your project must be a git repo. Run `git init` in the project root.

### Codex/Gemini not detected

Run `forge sync` — this re-detects tools. Make sure `codex` and `gemini` are in your PATH.

### All tasks blocked, nothing happening

Check dependencies. If T-001 failed and everything depends on it, nothing else can run. Fix T-001 first.

### Brain timeout on plan generation

Large specs can take 2+ minutes with reasoning models. The default timeout is 120 seconds. For very large specs, consider splitting into sections or using `gpt-4.1` instead of `gpt-5`.

---

## Typical Session (5 minutes)

```bash
cd my-project
forge init                   # 2 seconds
vim SPEC.md                  # Write your vision
forge plan --generate        # 30-90 seconds (AI decomposes spec)
forge status                 # Review the plan
forge start                  # Launch all agents (runs for minutes to hours)
# ... go get coffee ...
forge status                 # Check progress
# Or for fully hands-off:
forge start --ceo            # Loop until 100% complete, auto-retry failures
```

---

## FAQ

**Q: Do I need all three AI tools?**
No. Forge works with any combination. If you only have Codex, all tasks assigned to Claude/Gemini will be blocked. Reassign them in the task JSON files, or use `forge start --agent codex`.

**Q: Does Forge cost money?**
Forge itself is free and MIT-licensed. The AI tools have their own billing:
- Plan generation requires an `OPENAI_API_KEY` (or use the free rule-based brain)
- Claude headless mode uses your Max subscription (do NOT set `ANTHROPIC_API_KEY`)
- Codex requires a ChatGPT Pro subscription
- Gemini requires a Google AI Pro subscription

**Q: Can I edit the plan manually?**
Yes. Tasks are JSON files in `.forge/tasks/`. Edit `assigned_to`, `depends_on`, `status`, or anything else. Forge reads them fresh each time.

**Q: What if an agent writes bad code?**
Review the output in `.forge/results/`. Reset the task to `pending`, adjust the description or acceptance criteria, and re-run. The code is in your git repo — you can always `git diff` or `git reset`.

**Q: Can I use this with MCP (Model Context Protocol)?**
Yes. Add to `.mcp.json`:

```json
{
  "mcpServers": {
    "forge": {
      "type": "stdio",
      "command": "forge",
      "args": ["mcp", "--project", "."]
    }
  }
}
```

Then your AI tool can call `forge_get_tasks()`, `forge_claim_task()`, etc. natively.
