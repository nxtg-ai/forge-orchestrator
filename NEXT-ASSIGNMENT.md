# Assignment: RESEARCH-001 — CLI Subscription Gating: Exhaustive Risk Analysis

> **Scope:** RESEARCH ONLY. No code changes. Produce a comprehensive research document.
> **Output:** `docs/research/cli-subscription-gating-analysis.md`
> **Priority:** CRITICAL — This research gates whether forge-orca can safely orchestrate subscription-based CLI tools at all.

## Context

Forge-orca orchestrates AI coding agents (Claude Code, OpenAI Codex CLI, Google Gemini CLI) by spawning headless CLI processes with `-p` (prompt) flags. During SynApps dogfood (Feb 15, 2026), we discovered that **automated/headless CLI usage is rate-limited differently than interactive TUI usage**:

- **Codex CLI:** Hit `usage_limit_reached` after 24 tasks in ~2 hours. Error payload: `plan_type: "team"`, includes `resets_at` timestamp. Interactive TUI usage at similar volume does NOT trigger this.
- **Gemini CLI:** Hit `429 No capacity available for model gemini-2.5-pro` after only 2 tasks. This appears to be server capacity, not per-user quota.
- **Claude Code:** No rate limit hit (completed 3/4 tasks). But Anthropic's usage policy for Max subscription is unclear on automated/scripted usage.

**The founder's position:** "If I fuck over my users with bans.. I'm going to be very very very upset.. so it will not happen. TOO DANGEROUS."

**Current status:** Subscription-based orchestration is TABLED until this research is complete and validated.

## Research Questions (Answer ALL of these exhaustively)

### 1. Terms of Service Analysis
For each provider (Anthropic/Claude, OpenAI/Codex, Google/Gemini):
- What do the ToS/AUP say about automated/scripted usage of CLI tools?
- Is there an explicit prohibition on headless/non-interactive usage?
- Is there a distinction between "using the CLI" and "orchestrating the CLI programmatically"?
- What are the stated consequences for ToS violations? (warning, throttle, temp ban, permanent ban, account termination?)
- Are there any safe harbors for developer tools that wrap CLIs?

### 2. Rate Limit Architecture
For each provider:
- What are the exact rate limits for subscription tiers? (messages/hour, messages/5h, RPM, TPM)
- How are rate limits different between interactive and headless modes?
- Are rate limits per-user, per-API-key, per-IP, per-device, or per-session?
- What HTTP headers or error payloads expose rate limit state? (`X-RateLimit-*`, `Retry-After`, etc.)
- Is there a "shadow ban" or "soft throttle" that degrades quality before hard-blocking?
- Do rate limits reset on a rolling window or fixed window?

### 3. Subscription Tiers
For each provider:
- What subscription tiers exist? (Free, Pro, Team, Max, Enterprise, etc.)
- What are the exact quotas per tier?
- Which tiers explicitly support or allow API/automation access?
- Is there an "enterprise" or "business" tier that removes automation restrictions?
- What is the price delta between subscription and API-key usage for equivalent volume?

### 4. Detection Mechanisms
For each provider:
- How does the provider detect automated vs interactive usage? (timing patterns, User-Agent, process tree, PTY detection, stdin/stdout pipe detection?)
- Does the CLI phone home with usage telemetry that could flag automation?
- Are there known fingerprinting techniques? (e.g., checking if stdin is a TTY)
- What does the CLI binary actually send to the server? (inspect network traffic if possible)

### 5. Prior Art & Community Research
- Search arXiv for papers on: API rate limiting, LLM access gating, automated tool orchestration, fair-use detection
- Search GitHub for: other orchestrators that wrap Claude/Codex/Gemini CLIs, rate limit discussions, ban reports
- Search forums/Reddit/HN for: user reports of bans, throttling, or account actions from automated CLI usage
- Search provider developer forums for: official guidance on CLI automation
- Document any open-source projects that do what we're doing and their approach to rate limits

### 6. Safe Path Analysis
- Is there a provider-blessed way to orchestrate CLI tools at scale? (MCP? API? Enterprise agreements?)
- What would an "API-only" mode cost per month for typical forge usage? (estimate 50-200 tasks/day across 3 providers)
- Is the MCP (Model Context Protocol) server approach a viable alternative to CLI spawning?
- Could we use official SDKs/APIs instead of CLI wrappers? What capabilities would we lose?
- What is the business case for each provider to allow vs block CLI orchestration?

### 7. Risk Matrix
Create a risk matrix for each provider:
- **Probability of detection** (how likely is it that automated usage is detected?)
- **Probability of action** (if detected, how likely is enforcement?)
- **Severity of action** (warning vs throttle vs temp ban vs permanent ban vs legal)
- **Reversibility** (can the user recover from the action?)
- **User impact** (what happens to the user's workflow if their account is actioned?)

### 8. Recommendations
Based on ALL of the above:
- Should forge-orca support subscription-mode orchestration AT ALL?
- If yes, under what conditions? (max tasks/hour, pacing, provider restrictions)
- If no, what alternative architecture should we pursue?
- What disclaimers/warnings should we show users?
- What is our legal exposure if a user gets banned while using forge?

## Source Requirements

- **arXiv papers:** At minimum 3-5 relevant papers on API rate limiting, LLM orchestration, or fair-use detection
- **Official documentation:** Direct links to each provider's ToS, AUP, rate limit docs, pricing pages
- **GitHub issues/discussions:** Direct links to relevant threads about CLI automation limits
- **Forum posts:** Direct links to user reports of bans or throttling
- **Code analysis:** If you can inspect CLI binary behavior (network calls, telemetry), document findings
- **Every claim must have a source URL or citation**

## Output Format

```markdown
# CLI Subscription Gating: Exhaustive Risk Analysis

## Executive Summary
[2-3 paragraph summary with clear GO/NO-GO recommendation]

## 1. Terms of Service Analysis
### 1.1 Anthropic (Claude Code)
### 1.2 OpenAI (Codex CLI)
### 1.3 Google (Gemini CLI)

## 2. Rate Limit Architecture
[...]

## 3. Subscription Tiers
[...]

## 4. Detection Mechanisms
[...]

## 5. Prior Art & Community Research
### 5.1 Academic Papers
### 5.2 Open Source Projects
### 5.3 Community Reports

## 6. Safe Path Analysis
[...]

## 7. Risk Matrix
[Table format per provider]

## 8. Recommendations
### 8.1 GO/NO-GO Decision
### 8.2 If GO: Safety Requirements
### 8.3 If NO-GO: Alternative Architecture
### 8.4 Legal Considerations

## Sources
[Numbered reference list with URLs]
```

## IMPORTANT NOTES

- This is RESEARCH ONLY. Do NOT write any code.
- Do NOT make assumptions — cite sources for every claim.
- If information is unavailable or unclear, say so explicitly rather than guessing.
- Be pessimistic in risk assessment — we'd rather over-estimate risk than under-estimate.
- The founder will read this personally and make a go/no-go decision based on it.
- Create the output file at `docs/research/cli-subscription-gating-analysis.md`
- Create the `docs/research/` directory if it doesn't exist.

---

**CHECKPOINT: The document must be comprehensive (3000+ words minimum), cite real sources, and provide a clear recommendation.**
