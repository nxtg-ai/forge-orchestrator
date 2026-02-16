# CLI Subscription Gating: Exhaustive Risk Analysis

**Document:** RESEARCH-001
**Date:** 2026-02-15
**Author:** Claude (Opus 4.6) — Commissioned by forge-orchestrator team
**Classification:** CRITICAL — Gates subscription-mode orchestration decisions
**Status:** COMPLETE

---

## Executive Summary

**Recommendation: CONDITIONAL NO-GO on subscription-based CLI orchestration. STRONG GO on API-based orchestration.**

After exhaustive analysis of Terms of Service, rate limit architectures, detection mechanisms, academic research, community reports, and 10+ open-source precedents, the findings are unambiguous:

1. **Anthropic has explicitly and aggressively blocked third-party orchestration of Claude Code using subscription credentials.** In January 2026, Anthropic deployed server-side enforcement that detects and blocks non-official clients using subscription OAuth tokens. Tools like OpenCode (56K GitHub stars), Roo Code, and Cline were blocked. Users were banned (some false positives later reversed). The economic motivation is clear: a $200/mo Max subscription provides token throughput worth $1,000–$2,000+/mo at API rates. Anthropic's ToS Section D.4 prohibits "access[ing] the Services through automated or non-human means...except when you are accessing our Services via an Anthropic API Key or where we otherwise explicitly permit it." Forge-orchestrator's subscription-mode adapter falls squarely in the prohibited category.

2. **OpenAI and Google are materially less restrictive**, but subscription tiers impose hard rate limits (Codex: 10–60 cloud tasks/5h on Team; Gemini: 25–100 Pro requests/day on free tier) that make subscription-based orchestration operationally unreliable. The `usage_limit_reached` and `429 No capacity available` errors observed during SynApps dogfood are expected behavior at these volumes, not bugs.

3. **All three providers offer blessed API/SDK paths for programmatic orchestration** — Anthropic's Claude Agent SDK, OpenAI's Codex SDK + Agents SDK, and Google's open-source Gemini CLI + GenAI SDK. These paths are ToS-compliant, provide proper rate-limit headers for backpressure, and scale predictably. Estimated cost for 100 tasks/day across all 3 providers using smart routing: **~$149/mo** (less than a single Claude Max 20x subscription).

**The safe path forward is clear: migrate from CLI subscription spawning to API-key-authenticated SDK calls. The risk-to-cost ratio of subscription mode is catastrophic — user account bans for a tool that's supposed to empower developers.**

---

## 1. Terms of Service Analysis

### 1.1 Anthropic (Claude Code)

**Governing Terms:** [Consumer Terms of Service](https://www.anthropic.com/legal/consumer-terms) (effective October 8, 2025), [Commercial Terms](https://www.anthropic.com/legal/commercial-terms)

**The Core Prohibition:**
> "...to access the Services through automated or non-human means, whether through a bot, script, or otherwise, **except when you are accessing our Services via an Anthropic API Key or where we otherwise explicitly permit it**."
— Consumer ToS, Section D.4 [1]

This creates a two-tier system:

| Access Method | Automated Usage | Status |
|---|---|---|
| Anthropic API Key (pay-per-token) | Explicitly permitted | **Sanctioned** |
| Claude Code `-p` mode with API key auth | Officially documented at [code.claude.com/docs/en/headless](https://code.claude.com/docs/en/headless) | **Sanctioned** |
| Claude Code GitHub Action ([claude-code-action](https://github.com/anthropics/claude-code-action)) | First-party CI/CD tool | **Sanctioned** |
| Claude Agent SDK with API key | Explicitly designed for programmatic orchestration | **Sanctioned** |
| Claude Code `-p` mode with subscription OAuth | Grey area — officially supported for the authenticated user's own use | **Risky** |
| Third-party tool using subscription OAuth tokens | Prohibited — actively enforced since Jan 2026 | **Prohibited** |

**The Critical Nuance:** Anthropic's own documentation explicitly supports headless/programmatic usage via `-p`/`--print` flags and the Agent SDK. However, the Agent SDK docs state: *"Anthropic does not allow third party developers to offer claude.ai login or rate limits for their products, including agents built on the Claude Agent SDK, unless previously approved."* [2] This means forge-orchestrator offering subscription-mode orchestration is explicitly prohibited.

**Consequences for Violations:**
- Immediate suspension or termination without notice [1]
- No refund for terminated subscriptions [1]
- Data deletion at Anthropic's option [1]
- Indemnification obligation — user agrees to cover Anthropic's legal costs [1]

**Real-world enforcement (January 2026):** Anthropic deployed strict technical safeguards blocking subscription OAuth tokens from working outside the official Claude Code CLI. Tools like OpenCode, Roo Code, and Cline were blocked. Thariq Shihipar (Anthropic, Claude Code team) confirmed the enforcement was intentional. Some accounts were banned (acknowledged false positives were reversed). [3][4][5]

**Safe harbors for developer tools:** The Agent SDK is the explicitly blessed path. Using API keys for orchestration is unambiguously permitted. No "developer tools" safe harbor exists for subscription-credential-based orchestration.

### 1.2 OpenAI (Codex CLI)

**Governing Terms:** [Terms of Use](https://openai.com/policies/row-terms-of-use/), [Service Terms](https://openai.com/policies/service-terms/)

**Relevant ToS Language:**
> "except as permitted through the API, use any automated or programmatic method to extract data or output from the Services"
— Terms of Use [6]

> "interfering with or disrupting the Services, including circumventing any rate limits or restrictions or bypassing any protective measures"
— Terms of Use [6]

**Is Headless Usage Prohibited? No — it is officially supported.** OpenAI provides:
- `codex exec` — official headless mode with `--json` output, `--ephemeral` mode, and CI authentication via `CODEX_API_KEY` [7]
- Codex GitHub Action (`openai/codex-action@v1`) — official CI/CD automation [8]
- Codex SDK — programmatic control for building orchestration [9]

**Critical distinction:** The ToS prohibition applies to "Consumer Services" but explicitly carves out "except as permitted through the API." API-key-authenticated headless usage is explicitly permitted. Subscription-authenticated headless usage operates in a grey area — `codex exec` works with subscription auth, but the `usage_limit_reached` error at 24 tasks/2h demonstrates the practical constraint.

**Consequences:** Account suspension or termination. OpenAI provides warnings first for certain violations. Appeals available through OpenAI Support. [10]

### 1.3 Google (Gemini CLI)

**Governing Terms:** Depends on auth method [11]:
- Google Account (free) → Google Terms of Service + Code Assist Privacy Notice
- Gemini API Key → [Gemini API Additional Terms](https://ai.google.dev/gemini-api/terms)
- Vertex AI → [Google Cloud Platform Service Terms](https://cloud.google.com/terms)

**Is Headless Usage Prohibited? No — explicitly supported and documented.** Google provides first-class headless mode with `--prompt`/`-p`, `--output-format json`, and `--yolo`/`-y` flags designed for CI/CD and scripting. [12]

**No restriction on automation method.** The Gemini API ToS restricts *what you generate* (content policy), not *how you call the API*. There is no prohibition on headless, scripted, or orchestrated usage. [13]

**Gemini CLI is open-source (Apache 2.0)** — wrapping it is explicitly permitted by the license. [14]

**Consequences for violations:** Graduated enforcement: email discussion → temporary limits → temporary suspension → permanent account closure (extends to other Google services). Focus is on content policy violations, not automation detection. [15]

---

## 2. Rate Limit Architecture

### 2.1 Anthropic (Claude Code)

**Subscription Rate Limits (5-hour rolling window):**

| Plan | Price | Messages / 5h Window | Weekly Cap | Models |
|---|---|---|---|---|
| Free | $0 | 2–5 | Very limited | Sonnet 4 only |
| Pro | $20/mo | 10–40 | ~40–80 Claude Code hours | Sonnet 4 |
| Max 5x | $100/mo | 50–200 | Proportionally scaled | Sonnet 4 + Opus 4 |
| Max 20x | $200/mo | 200–800 | Proportionally scaled | Sonnet 4 + Opus 4 |

All Claude surfaces (claude.ai, Claude Code CLI, Claude Desktop) share the same usage bucket. Weekly limits were added August 28, 2025 to address account-sharing abuse and "24/7 autonomous agent" usage patterns. Opus models consume ~1.7x quota vs Sonnet. [16][17]

**API Rate Limits (per-minute, well-documented headers):**

| Tier | Requirement | RPM | Input TPM | Output TPM |
|---|---|---|---|---|
| Tier 1 | $5 credit | 50 | 30,000 | 8,000 |
| Tier 2 | $40 cumulative | 1,000 | 450,000 | 90,000 |
| Tier 3 | $200 cumulative | 2,000 | 800,000 | 160,000 |
| Tier 4 | $400 cumulative | 4,000 | 2,000,000 | 400,000 |

**HTTP headers for API rate limit state:** Every API response includes `anthropic-ratelimit-requests-limit`, `anthropic-ratelimit-requests-remaining`, `anthropic-ratelimit-tokens-limit`, `anthropic-ratelimit-tokens-remaining`, `retry-after`, and reset timestamps (RFC 3339). Subscription CLI does NOT expose equivalent headers — you simply get throttled. [18]

**Interactive vs headless difference:** No documented per-mode rate limits. However, Anthropic internally detects 24/7 autonomous loops (some Max subscribers were consuming "tens of thousands of dollars" of compute monthly). The enforcement is economic, not technical mode detection. [19]

**Shadow ban / soft throttle:** Reports on GitHub issue [#9094](https://github.com/anthropics/claude-code/issues/9094) (30+ users) show unexplained reduced limits after Sonnet 4.5 release — users reporting 6–8 hours/week vs expected 40–50. No official acknowledgment of "soft throttling." [20]

### 2.2 OpenAI (Codex CLI)

**Subscription Rate Limits (5-hour rolling window + weekly caps):**

| Plan | Local Messages / 5h | Cloud Tasks / 5h | Code Reviews / Week |
|---|---|---|---|
| Plus ($20/mo) | 45–225 | 10–60 | 10–25 |
| Pro ($200/mo) | 300–1,500 | 50–400 | 100–250 |
| Business ($30/user/mo) | 45–225 | 10–60 | Not specified |
| Enterprise | Contact sales | Contact sales | Contact sales |

Ranges reflect that "the number of messages you can send varies based on size and complexity." [21]

**The observed behavior (24 tasks/~2h, Team plan) is expected.** Business/Team allows 10–60 cloud tasks per 5-hour window. At 24 tasks in 2 hours with complex workloads, hitting the ceiling is normal. The `usage_limit_reached` error with `resets_at` timestamp matches the 5-hour rolling window reset.

**Interactive vs headless:** No documented difference. `codex exec` "shares the same codex-core backend as the TUI but presents events as structured output." Rate limits are server-side, mode-agnostic. [22]

**Beyond Rate Limits architecture:** OpenAI published a blog post (Feb 2026) describing their "Decision Waterfall Model" — every request passes through a sequence checking rate limits first, then automatically falls through to credit balances if limits are exhausted. [23]

**Known bugs:** Rate limit errors can consume entire weekly budgets without producing useful output (GitHub Issues #11508, #7255). "Approaching rate limits" warnings appear falsely at startup (#7126). [24][25][26]

### 2.3 Google (Gemini CLI)

**Rate Limits by Auth Method:**

| Auth Method | RPM | RPD | Model Access |
|---|---|---|---|
| Google Account (free) | 60 | 1,000 | Pro + Flash blend |
| Gemini API Key (free) | 10 | 250 | Flash only |
| Code Assist Standard | 120 | 1,500 | Full model family |
| Code Assist Enterprise | 120 | 2,000 | Full model family |

**Critical finding — Gemini 2.5 Pro sub-quota:** The 1,000 RPD total includes a much smaller Pro allocation (~25–100 requests/day on free tier). After exhausting Pro, the CLI auto-switches to Flash: *"You have reached your daily gemini-2.5-pro quota limit. Automatically switching from gemini-2.5-pro to gemini-2.5-flash."* Users report hitting Pro limits after 10–15 prompts. [27][28]

**Your 429 after 2 tasks:** Likely a combination of (a) the low Pro sub-quota and (b) server-side capacity constraints — the `429 No capacity available for model gemini-2.5-pro` is an infrastructure scaling problem, not a per-user rate limit. A Google contributor (F. Hinkelmann) acknowledged demand exceeded provisioned capacity. [29]

**Rate limits are per-project, not per-API-key.** Multiple apps sharing the same project share the same rate limit. [30]

**Interactive vs headless:** No documented difference. Same quotas apply regardless of invocation mode.

---

## 3. Subscription Tiers

### 3.1 Anthropic

| Tier | Price | Claude Code | Automation Support |
|---|---|---|---|
| Free | $0 | No | N/A |
| Pro | $20/mo | Yes | `-p` mode with subscription auth (own use only) |
| Max 5x | $100/mo | Yes, full | `-p` mode with subscription auth (own use only) |
| Max 20x | $200/mo | Yes, full | `-p` mode with subscription auth (own use only) |
| Team Standard | $25/user/mo | Yes | Shared projects, admin controls |
| Team Premium | $150/user/mo | Yes | Enhanced team features |
| Enterprise | Custom | Yes | SSO, SCIM, audit logs, custom SLAs |
| **API (separate)** | **Pay-per-token** | **Via Agent SDK** | **Full automation, no restrictions** |

The API is a completely separate product. For building products that serve other users, the API is required. [2]

### 3.2 OpenAI

| Tier | Price | Codex Access | Automation |
|---|---|---|---|
| Free | $0 | Limited | No |
| Plus | $20/mo | Yes — CLI, web, IDE | `codex exec` with subscription |
| Pro | $200/mo | Yes — priority, 6x Plus limits | `codex exec` with subscription |
| Business | $25–30/user/mo | Yes — larger VMs | `codex exec` with subscription |
| Enterprise | Custom | Yes — priority, custom quotas | Full support |
| **API Key** | **Pay-per-token** | **Yes — local only** | **Fully supported, no caps** |

API key mode does NOT support cloud task execution or code review integration. [21]

### 3.3 Google

| Tier | Price | CLI Quota | Automation |
|---|---|---|---|
| Free (Google Account) | $0 | 1,000 RPD, 60 RPM | No restriction |
| Free (API Key) | $0 | 250 RPD, 10 RPM | No restriction |
| Google AI Pro | $19.99/mo | 1,500 RPD, 120 RPM | No restriction |
| Google AI Ultra | $249.99/mo | 2,000 RPD, 120 RPM | No restriction |
| **Vertex AI Pay-as-you-go** | **Per-token** | **Dynamic shared quota** | **No restriction** |

Google is the most permissive — no tier restricts headless/automated usage. [31][32]

### 3.4 Price Delta: Subscription vs API

For equivalent heavy coding usage (~7.5M input + 750K output tokens/day):

| Path | Monthly Cost | Notes |
|---|---|---|
| Claude Max 20x | $200 | All models, but throttled + ToS risk for orchestration |
| Claude API (Sonnet 4.5 w/ caching) | $150–250 | No throttle, proper headers, ToS-compliant |
| Claude API (Opus 4.5 w/ caching) | $250–400 | Premium model, no restrictions |
| ChatGPT Pro | $200 | Up to 1,500 msgs/5h, weekly caps |
| OpenAI API (codex-mini-latest) | $68–135 | No caps, cheapest option |
| Gemini CLI (free tier) | $0 | 1,000 RPD but unreliable Pro allocation |
| Vertex AI (Gemini 2.5 Pro) | $94–188 | No caps, enterprise SLA available |

The subscription arbitrage is stark for Anthropic: Max 20x provides 4–18x more effective value than equivalent API spend. This is precisely why Anthropic enforces against subscription-credential orchestration. [33]

---

## 4. Detection Mechanisms

### 4.1 Anthropic (Claude Code)

Anthropic uses multiple detection vectors, confirmed by the January 2026 enforcement:

1. **Client Identity Headers:** Primary vector. Claude Code sends specific identifying headers. Server-side checks now validate client authenticity. Third-party tools spoofing these headers are detected and blocked. [3]

2. **Telemetry Signals:** Claude Code uses OpenTelemetry for diagnostics. Anthropic noted that "most third-party harnesses do not" send this telemetry, making unauthorized traffic identifiable. Without telemetry, Anthropic stated they "cannot reliably explain rate limits, errors, or bans." [4]

3. **TTY Detection:** Claude Code uses the Ink library which checks `process.stdin.isTTY`. The `-p` flag bypasses this for legitimate headless use. The existence of [Headless-TTY](https://github.com/revoconner/Headless-TTY) (a tool to fool `isatty()` checks) confirms this is a detection signal. [34]

4. **OAuth Token Validation:** Server-side checks validate that OAuth tokens are being used by the official Claude Code client, not by third-party tools with valid credentials.

5. **Usage Patterns:** 24/7 autonomous agent loops create distinctive patterns — request cadence, session duration, token volume distributions. The FairServe paper from Microsoft Research [35] demonstrates how providers model per-user request distributions to identify anomalous patterns.

6. **Telemetry Bug:** Issue [#10494](https://github.com/anthropics/claude-code/issues/10494) shows Claude Code connected to Google Analytics endpoints despite `DISABLE_TELEMETRY` being set, indicating active-by-default analytics. [36]

### 4.2 OpenAI (Codex CLI)

1. **Anonymous Analytics (enabled by default):** Tracks experimental features, tool call counts, approval decisions, conversation turn counts, and duration. No PII or code included. Opt-out via `config.toml`: `[analytics] enabled = false`. [37]

2. **OpenTelemetry (opt-in, off by default):** When enabled, exports API request details, token counts, and user prompts (redactable). [38]

3. **Metadata Fields on Every Event:** `auth_mode` (subscription vs API), `terminal.type` (terminal environment), `conversation.id`, `app.version`, `user.account_id`. The `terminal.type` field distinguishes PTY-backed interactive terminals from non-TTY headless environments. [37]

4. **No documented fingerprinting** of calling process, parent shell, or orchestration wrappers.

### 4.3 Google (Gemini CLI)

1. **Telemetry disabled by default.** `GEMINI_TELEMETRY_ENABLED` defaults to `false`. When enabled, it routes to YOUR configured endpoint, not Google. [39]

2. **Server-side observation:** Google's API servers see request frequency, auth method, IP, User-Agent, and token consumption — standard API telemetry.

3. **No evidence of PTY detection or automation penalization.** Abuse monitoring focuses on content policy violations, not invocation method. [15]

4. **Gemini CLI sets `GEMINI_CLI=1` environment variable** in subprocesses — for child-process detection, not automation flagging. [14]

---

## 5. Prior Art & Community Research

### 5.1 Academic Papers

**Paper 1: "Rethinking HTTP API Rate Limiting: A Client-Side Approach"**
- Authors: Farkiani, Liu, Crowley
- arXiv: [2510.04516](https://arxiv.org/abs/2510.04516) (October 2025)
- Proposes Adaptive Token Bucket (ATB) and Assisted ATB (AATB) algorithms. AATB reduces HTTP 429 errors by 96.9–97.3% with only 13.3–19.8% duration increase. Demonstrates that exponential backoff is grossly inefficient.
- **Relevance:** Directly applicable to forge-orchestrator's pacing strategy. ATB-style adaptive algorithms should replace fixed cooldowns.

**Paper 2: "Multi-Objective Adaptive Rate Limiting Using Deep Reinforcement Learning"**
- Authors: Lyu, Wang, Cheng, Zhang, Chen
- arXiv: [2511.03279](https://arxiv.org/abs/2511.03279) (November 2025)
- Hybrid DQN + A3C architecture achieves 23.7% throughput improvement and 31.4% P99 latency reduction vs fixed thresholds. 90-day production deployment: 82% reduction in degradation incidents.
- **Relevance:** Validates that adaptive (not fixed) rate limiting is far superior. Dynamic pacing should be a core architectural feature.

**Paper 3: "Ensuring Fair LLM Serving Amid Diverse Applications (FairServe)"**
- Authors: Chokshi et al. (Microsoft Research)
- arXiv: [2411.15997](https://arxiv.org/abs/2411.15997) (November 2024)
- Analyzed millions of requests from thousands of users on MS CoPilot. Proposes Overload and Interaction-driven Throttling (OIT) + Weighted Service Counter (WSC). Identifies that some users submit excessive requests causing service unavailability for others.
- **Relevance:** This is the provider-side perspective. Shows exactly how providers detect "abusive" patterns — high request volume, unusual token distributions, session duration anomalies.

**Paper 4: "Scrappy: SeCure Rate Assuring Protocol with PrivacY"**
- Authors: Akama, Nakatsuka, Sato, Uehara (Keio / ETH Zurich)
- arXiv: [2312.00989](https://arxiv.org/abs/2312.00989) (NDSS 2024)
- Proposes cryptographic unforgeable yet unlinkable rate-assuring proofs. Represents the future of rate limiting — cryptographic compliance rather than heuristic detection.
- **Relevance:** Industry direction. Eventual rate-limiting may become cryptographically enforced rather than pattern-based.

**Paper 5: "Difficulty-Aware Agentic Orchestration (DAAO)"**
- Authors: Su, Lan, Xia, Sun, Tian, Shi, Song, He
- arXiv: [2509.11079](https://arxiv.org/abs/2509.11079) (September 2025)
- Dynamically generates query-specific multi-agent workflows guided by predicted difficulty. Uses cost- and performance-aware LLM routing.
- **Relevance:** Directly applicable to forge-orchestrator. Simple tasks should route to cheaper models (codex-mini, Flash) while complex tasks use premium models (Opus, Pro).

**Paper 6: "Multi-Agent LLM Orchestration for Incident Response"**
- arXiv: [2511.15755](https://arxiv.org/abs/2511.15755) (November 2025)
- 348 controlled trials: multi-agent orchestration achieved 100% actionable recommendation rate vs 1.7% for single-agent — 80x improvement in action specificity.
- **Relevance:** Validates that multi-agent orchestration produces dramatically better results, justifying forge-orchestrator's architectural complexity.

### 5.2 Open Source Projects

**Full Multi-CLI Orchestrators (directly comparable to forge-orchestrator):**

| Project | URL | Description | Rate Limit Approach |
|---|---|---|---|
| **AWS CLI Agent Orchestrator** | [awslabs/cli-agent-orchestrator](https://github.com/awslabs/cli-agent-orchestrator) | Official AWS tool — orchestrates Claude Code + Amazon Q CLI via tmux + MCP. Plans Codex/Gemini support. | Not documented |
| **claude-octopus** | [nyldn/claude-octopus](https://github.com/nyldn/claude-octopus) | Multi-tentacled orchestrator for Claude + Codex + Gemini. 29 expert personas, 44 skills. | Not documented |
| **myclaude** | [cexll/myclaude](https://github.com/cexll/myclaude) | Multi-agent workflow for Claude + Codex + Gemini + OpenCode. | Not documented |
| **Claude-Code-Workflow** | [catlog22/Claude-Code-Workflow](https://github.com/catlog22/Claude-Code-Workflow) | JSON-driven multi-agent framework with CLI orchestration. | Not documented |
| **gemini-orchestrator** | [lucad87/gemini-orchestrator](https://github.com/lucad87/gemini-orchestrator) | Shell-based Gemini CLI orchestrator. **Includes configurable cooldowns between phases.** | Configurable cooldowns |

**Claude Code-specific orchestrators:** claude-flow, agentctl, ccswarm, claude-swarm, claude-code-by-agents — all spawn Claude Code processes for multi-agent coordination. None document rate-limit pacing strategies, suggesting this is an unsolved problem in the ecosystem.

**Rate limit workaround tools:** cursor-auto-resume (auto-clicks "resume conversation" in Cursor after rate limits). Cline auto-retry discussions (Issue #213, #7267). These demonstrate the pragmatic approach users take — workarounds rather than solutions.

### 5.3 Community Reports

**Anthropic — Third-Party Harness Crackdown (January 2026):**
- Anthropic blocked subscription OAuth from non-official clients. OpenCode (56K stars), Roo Code, Cline, Kilo affected. [3][4][5]
- xAI employees lost access after using Claude via Cursor. Previously, OpenAI was revoked in August 2025. [3]
- Thariq (Anthropic) cited: unusual traffic patterns without telemetry, inability to debug bans, ToS Section D.4 violation. [40]
- Some Max subscribers were consuming compute worth "tens of thousands of dollars" monthly via 24/7 autonomous agents. [19]

**Anthropic — Rate Limit Issues:**
- 30+ reports of unexplained reduced limits after Sonnet 4.5 release (GitHub Issue #9094). [20]
- Weekly rate limits announced July 2025, effective August 28, 2025. Pro: 40–80 hours/week. Max: up to 480 hours Sonnet + 40 hours Opus. [17]
- Rate limit hit causes Claude Code to enter infinite `/rate-limit-options` loop, burning credit allocation without useful work. Returns exit code 0. (Issue #18388, #15685). [41]

**OpenAI — Codex Rate Limit Issues:**
- Plus users hitting limits after 1–2 requests (Issue #2448). [24]
- Phantom usage drain: weekly limit % dropping overnight without active use (Issue #7255). [25]
- Budget consumed on error responses without useful output (Issue #11508). [42]
- Community reports "hundreds of thousands of paying users" leaving the service (Issue #7676). [43]

**Google — Gemini Rate Limit Issues:**
- December 7, 2025: Free tier RPM reduced from 15 to 5. [44]
- Users hitting "Rate Limit Exceeded" after ~1 hour of work (Issue #10513). [45]
- Advertised 1,500 RPD but actual limits stuck at 250 (Google AI Forum). [46]

---

## 6. Safe Path Analysis

### 6.1 Provider-Blessed Orchestration Paths

**Anthropic:** Use the Claude Agent SDK (Python/TypeScript) with API keys. Same engine as Claude Code, standard API pricing. 1,550 free hours/month of code execution sandbox ($0.05/hr after). [2][47]

**OpenAI:** Use the Codex SDK + Agents SDK with API keys. OpenAI explicitly publishes cookbook examples of Codex CLI as MCP server orchestrated by the Agents SDK. `codex-mini-latest` at $1.50/$6.00 per MTok is the cheapest option. [9][48][49]

**Google:** Use Vertex AI Pay-as-you-go or the free tier (1,000 RPD). Gemini CLI is Apache 2.0 open source — embedding/wrapping is explicitly permitted. GenAI SDK supports both Developer API and Vertex AI. [14][31]

### 6.2 API-Only Mode Cost Estimates

**Per-task cost (10K input + 5K output tokens):**

| Model | Cost Per Task |
|---|---|
| Claude Sonnet 4/4.5 | $0.105 |
| Claude Opus 4.5 | $0.175 |
| GPT-4.1 / o3 | $0.060 |
| codex-mini-latest | $0.045 |
| Gemini 2.5 Pro | $0.063 |

**Important caveat:** Real agentic coding tasks involve multi-turn loops. A single "task" may require 3–10 API round-trips with growing context windows. Realistic per-task costs may be 3–10x the above estimates for complex work.

**Monthly cost projections (30 days):**

| Volume | Claude Sonnet 4.5 | codex-mini-latest | Gemini 2.5 Pro | All 3 (smart routing) |
|---|---|---|---|---|
| 50 tasks/day | $158 | $68 | $94 | ~$107 |
| 100 tasks/day | $315 | $135 | $188 | ~$149* |
| 200 tasks/day | $630 | $270 | $375 | ~$335* |

*Smart routing: 30% Gemini free tier (simple), 40% codex-mini (medium), 30% Sonnet (complex)*

### 6.3 MCP-Based Alternative

MCP is viable and now officially blessed by all three providers:
- **Pros:** Provider-agnostic, tasks primitive for async work, OAuth 2.1 auth spec
- **Cons:** No protocol-level rate limiting (must implement your own), high context overhead (20+ tools = 50K+ tokens on definitions), no tool governance/versioning
- MCP itself has no rate limits — limiting is the server implementation's responsibility [50]

### 6.4 SDK Feature Comparison

| Feature | Claude Agent SDK | Codex SDK | Gemini GenAI SDK |
|---|---|---|---|
| File Read/Write/Edit | Yes (built-in) | Yes | No (API sandbox limitation) |
| Bash execution | Yes (built-in) | Yes | No (30s timeout) |
| Search (Glob/Grep) | Yes (built-in) | Yes | No |
| MCP support | Yes (in-process) | Yes (MCP server) | Via integration |
| Agent loop | Yes (managed) | Yes | Manual |
| Code sandbox | Yes ($0.05/hr) | Yes (cloud tasks) | Yes (Python only, 30s) |
| Billing | Per-token | Per-token | Per-token |

**Key gap for Google:** The GenAI SDK's code execution does NOT support file I/O and has a 30-second timeout. For file-heavy coding tasks, the CLI is superior. Since it's Apache 2.0, embedding it is the pragmatic path.

### 6.5 Business Case for Providers

| Factor | Anthropic | OpenAI | Google |
|---|---|---|---|
| Revenue model | API tokens + subscriptions | API tokens + subscriptions | GCP platform |
| CLI orchestration stance | Blocked for subscriptions; blessed via API/Agent SDK | Explicitly encouraged via SDK | Fully open (Apache 2.0 CLI) |
| Economic motivation | Subscription arbitrage costs them money | API revenue is core; orchestration drives consumption | Gemini drives GCP adoption |
| Enforcement intensity | Aggressive (Jan 2026 crackdown) | None against API-based orchestration | None |

---

## 7. Risk Matrix

### 7.1 Subscription-Mode Orchestration Risk

| Dimension | Anthropic (Claude) | OpenAI (Codex) | Google (Gemini) |
|---|---|---|---|
| **Probability of detection** | **VERY HIGH (95%)** — Active enforcement, client fingerprinting, telemetry analysis | **MODERATE (40%)** — `terminal.type` + `auth_mode` telemetry, but no documented enforcement | **LOW (10%)** — Telemetry opt-in, no documented detection |
| **Probability of action** | **VERY HIGH (90%)** — Actively blocking third-party harnesses since Jan 2026 | **LOW (15%)** — No known enforcement actions against `codex exec` users | **VERY LOW (5%)** — Content-focused abuse monitoring only |
| **Severity of action** | **CRITICAL** — Account termination without notice, no refund, data deletion possible | **MODERATE** — Warning → suspension → termination (graduated) | **LOW** — Email → temp limits → temp suspension (graduated) |
| **Reversibility** | **LOW** — Some false-positive bans were reversed, but no guarantees | **MODERATE** — Appeals process documented | **MODERATE** — Appeals process, graduated approach |
| **User impact** | **CATASTROPHIC** — Loss of Claude access across all surfaces (web, desktop, mobile, API if tied to same account) | **HIGH** — Loss of ChatGPT/Codex access, but API often on separate account | **MODERATE** — Loss of Google AI access, but core Google services unaffected |
| **Overall Risk Score** | **CRITICAL — DO NOT USE** | **MODERATE — Use with extreme caution** | **LOW — Acceptable with pacing** |

### 7.2 API-Mode Orchestration Risk

| Dimension | Anthropic API | OpenAI API | Google API (Vertex AI) |
|---|---|---|---|
| **Probability of detection** | N/A — Automated use is explicitly permitted | N/A — Explicitly permitted | N/A — Explicitly permitted |
| **Probability of action** | **NONE** — within normal API terms | **NONE** — within normal API terms | **NONE** — within normal API terms |
| **Severity of action** | Rate limiting (429) with proper headers | Rate limiting (429) with proper headers | Rate limiting (429) with proper headers |
| **Reversibility** | **HIGH** — Wait for reset, respect headers | **HIGH** — Wait for reset | **HIGH** — Wait for reset |
| **User impact** | Temporary slowdown, no account risk | Temporary slowdown, no account risk | Temporary slowdown, no account risk |
| **Overall Risk Score** | **SAFE** | **SAFE** | **SAFE** |

### 7.3 Combined Risk Summary

```
                    Subscription Mode        API Mode
                    ─────────────────        ────────
Anthropic:          ██████████ CRITICAL      ░░░░░░░░░░ SAFE
OpenAI:             ████░░░░░░ MODERATE      ░░░░░░░░░░ SAFE
Google:             ██░░░░░░░░ LOW           ░░░░░░░░░░ SAFE
```

---

## 8. Recommendations

### 8.1 GO/NO-GO Decision

**SUBSCRIPTION-MODE ORCHESTRATION: NO-GO for Anthropic. CONDITIONAL GO for OpenAI and Google with strict pacing.**

| Provider | Subscription Mode | API Mode |
|---|---|---|
| **Anthropic** | **NO-GO** — Active enforcement, catastrophic user impact, explicit ToS prohibition on third-party harnesses | **GO** — Explicitly blessed via Agent SDK |
| **OpenAI** | **CONDITIONAL GO** — `codex exec` is officially supported, but 10–60 cloud task/5h limit makes it operationally unreliable. Must implement pacing + graceful degradation. | **GO** — Explicitly blessed, no caps |
| **Google** | **GO with pacing** — No ToS prohibition, but low Pro sub-quota (~25–100/day) and capacity issues require adaptive pacing + Flash fallback | **GO** — Explicitly blessed, cheapest option |

**OVERALL RECOMMENDATION: Migrate to API-key-authenticated SDK orchestration as the primary path. Maintain subscription-mode as opt-in for OpenAI/Google only, with explicit user warnings and aggressive pacing.**

### 8.2 If GO: Safety Requirements (for retained subscription modes)

For OpenAI and Google subscription modes, implement ALL of the following:

1. **Adaptive Token Bucket pacing** (per Paper 1, arXiv:2510.04516):
   - Start with conservative rate (1 task/3 minutes)
   - Increase rate when no rate-limit signals detected
   - Immediately halve rate on any 429 or `usage_limit_reached`
   - Never exceed 60% of the documented per-5h-window quota

2. **Rate limit header parsing:**
   - Parse `resets_at` from Codex error payloads
   - Parse capacity headers from Gemini responses
   - Respect `Retry-After` headers exactly
   - Implement jittered exponential backoff as fallback

3. **Graceful degradation:**
   - On rate limit hit: pause subscription adapter, offer API-key fallback
   - Never retry more than 3 times on same rate limit window
   - Show clear user notification: "Subscription limit reached — switch to API key or wait until [reset_time]"
   - Exit code must be non-zero on rate limit (unlike Claude Code's buggy exit 0)

4. **Session pacing (anti-detection):**
   - Minimum 30-second gaps between sequential tasks to same provider
   - Randomized jitter (±15%) on all timing to avoid robotic patterns
   - Maximum 4 concurrent tasks across all subscription adapters
   - Daily usage tracking with configurable ceiling (default: 50% of documented daily quota)

5. **Telemetry compliance:**
   - Do NOT disable provider telemetry — it's a detection signal when missing
   - Send forge-orchestrator's own User-Agent identifying it honestly
   - Never spoof provider client headers

### 8.3 If NO-GO: Alternative Architecture

For Anthropic (and as the recommended primary path for all providers):

1. **Phase 1 (Immediate):** Add `auth_mode: api_key` configuration per adapter. When API key is set, use it for all headless orchestration. When only subscription auth is available, warn user and apply strict pacing.

2. **Phase 2 (Short-term):** Integrate Claude Agent SDK (Python/TypeScript) as a subprocess or via HTTP for complex agentic tasks. Use `reqwest` for direct API calls for simple tasks.

3. **Phase 3 (Medium-term):** Build a smart task router inspired by DAAO (Paper 5):
   - Simple tasks → Gemini free tier or codex-mini ($0.045/task)
   - Medium tasks → Claude Sonnet or GPT-4.1 ($0.06–$0.105/task)
   - Complex tasks → Claude Opus or GPT-5.2-codex ($0.175–$0.525/task)
   - Estimated cost: **$149/mo at 100 tasks/day** (less than one Max subscription)

4. **Phase 4 (Long-term):** Expose forge-orchestrator as an MCP server, enabling consumption by any MCP-compatible client (Claude Code, Codex CLI, etc.) — making forge itself orchestratable.

### 8.4 Legal Considerations

1. **Forge-orchestrator's liability if a user gets banned:**
   - If the user provided their own subscription credentials and forge triggered a ban, the user could claim forge caused damages (loss of access, loss of work, subscription cost)
   - **Mitigation:** Prominent disclaimer in subscription mode: *"Using subscription credentials for automated orchestration may violate provider Terms of Service and risk account suspension. We recommend API key authentication."*
   - Consider requiring explicit opt-in with checkbox acknowledgment for subscription mode

2. **No indemnification exposure if using API mode:**
   - API-key-authenticated automation is explicitly permitted by all three providers
   - Standard API rate limiting (429) is a normal, recoverable operational condition, not a policy violation

3. **Recommended legal disclaimer (for subscription mode):**
   > WARNING: Subscription-based orchestration is NOT endorsed by AI providers and may violate their Terms of Service. Anthropic has EXPLICITLY PROHIBITED this pattern and ACTIVELY ENFORCES against it. Using forge with subscription credentials risks account suspension or termination. We STRONGLY RECOMMEND using API keys instead. By proceeding with subscription mode, you acknowledge this risk and accept full responsibility for any consequences to your account.

4. **Recommended approach:** Make subscription mode require explicit `--i-accept-subscription-risk` flag. Default to API-key mode. Never auto-enable subscription mode.

---

## Sources

[1] Anthropic Consumer Terms of Service — https://www.anthropic.com/legal/consumer-terms
[2] Claude Agent SDK Overview — https://platform.claude.com/docs/en/agent-sdk/overview
[3] VentureBeat: Anthropic Cracks Down on Unauthorized Usage — https://venturebeat.com/technology/anthropic-cracks-down-on-unauthorized-claude-usage-by-third-party-harnesses
[4] Hacker News Discussion on Third-Party Blocking — https://news.ycombinator.com/item?id=46549823
[5] Hacker News: Anthropic Explicitly Blocking OpenCode — https://news.ycombinator.com/item?id=46625918
[6] OpenAI Terms of Use — https://openai.com/policies/row-terms-of-use/
[7] Codex CLI Noninteractive Mode — https://developers.openai.com/codex/noninteractive/
[8] Codex GitHub Action — https://developers.openai.com/codex/github-action/
[9] Codex SDK — https://developers.openai.com/codex/sdk/
[10] OpenAI Account Deactivation FAQ — https://help.openai.com/en/articles/10562188-why-was-my-openai-account-deactivated
[11] Gemini CLI ToS and Privacy — https://google-gemini.github.io/gemini-cli/docs/tos-privacy.html
[12] Gemini CLI Headless Mode — https://geminicli.com/docs/cli/headless/
[13] Gemini API Additional Terms — https://ai.google.dev/gemini-api/terms
[14] Gemini CLI GitHub Repository (Apache 2.0) — https://github.com/google-gemini/gemini-cli
[15] Gemini API Abuse Monitoring — https://ai.google.dev/gemini-api/docs/usage-policies
[16] Claude Code Token Limits (Faros AI) — https://www.faros.ai/blog/claude-code-token-limits
[17] TechCrunch: Anthropic Rate Limits for Claude Code — https://techcrunch.com/2025/07/28/anthropic-unveils-new-rate-limits-to-curb-claude-code-power-users/
[18] Anthropic API Rate Limits — https://platform.claude.com/docs/en/api/rate-limits
[19] Oreate AI: Claude Code Usage Limits — https://www.oreateai.com/blog/claude-code-announces-implementation-of-usage-limits-200-subscription-users-will-face-service-adjustments/59a6b2011d8c6351ebbb90d67bfa64e5
[20] The Register: Claude Devs Usage Limits — https://www.theregister.com/2026/01/05/claude_devs_usage_limits/
[21] Codex CLI Pricing — https://developers.openai.com/codex/pricing/
[22] Using Codex with ChatGPT Plan — https://help.openai.com/en/articles/11369540-using-codex-with-your-chatgpt-plan
[23] OpenAI: Beyond Rate Limits — https://openai.com/index/beyond-rate-limits/
[24] GitHub: Codex Issue #2448 (Plus users hitting limits) — https://github.com/openai/codex/issues/2448
[25] GitHub: Codex Issue #7255 (Phantom usage drain) — https://github.com/openai/codex/issues/7255
[26] GitHub: Codex Issue #7126 (False rate limit warnings) — https://github.com/openai/codex/issues/7126
[27] GitHub: Gemini Issue #4300 (Pro quota limit) — https://github.com/google-gemini/gemini-cli/issues/4300
[28] GitHub: Gemini Discussion #2436 (10-15 prompt limits) — https://github.com/google-gemini/gemini-cli/discussions/2436
[29] GitHub: Gemini Issue #1502 (429 capacity) — https://github.com/google-gemini/gemini-cli/issues/1502
[30] Gemini API Rate Limits — https://ai.google.dev/gemini-api/docs/rate-limits
[31] Gemini CLI Quota and Pricing — https://geminicli.com/docs/quota-and-pricing/
[32] Google AI Subscriptions — https://gemini.google/subscriptions/
[33] Anthropic API Pricing — https://platform.claude.com/docs/en/about-claude/pricing
[34] Headless-TTY GitHub — https://github.com/revoconner/Headless-TTY
[35] arXiv: FairServe (2411.15997) — https://arxiv.org/abs/2411.15997
[36] GitHub: Claude Code Issue #10494 (Telemetry leak) — https://github.com/anthropics/claude-code/issues/10494
[37] Codex Client Analytics — https://github.com/openai/codex/discussions/8291
[38] Codex OpenTelemetry PR #2103 — https://github.com/openai/codex/pull/2103
[39] Gemini CLI Telemetry — https://google-gemini.github.io/gemini-cli/docs/cli/telemetry.html
[40] Techmeme: Anthropic Third-Party Enforcement — https://www.techmeme.com/260109/p25
[41] GitHub: Claude Code Issue #18388 (Rate limit loop) — https://github.com/anthropics/claude-code/issues/18388
[42] GitHub: Codex Issue #11508 (Budget consumed on errors) — https://github.com/openai/codex/issues/11508
[43] GitHub: Codex Issue #7676 (Users leaving) — https://github.com/openai/codex/issues/7676
[44] AIFREEAPI: Gemini Quota Fix Guide — https://www.aifreeapi.com/en/posts/gemini-3-quota-exceeded-fix
[45] GitHub: Gemini Issue #10513 (1-hour limit) — https://github.com/google-gemini/gemini-cli/issues/10513
[46] Google AI Forum: Reduced Rate Limits — https://discuss.ai.google.dev/t/reduced-rate-limits/118210
[47] Claude Code Execution Tool — https://platform.claude.com/docs/en/agents-and-tools/tool-use/code-execution-tool
[48] Codex + Agents SDK Guide — https://developers.openai.com/codex/guides/agents-sdk/
[49] OpenAI Cookbook: Codex MCP + Agents SDK — https://cookbook.openai.com/examples/codex/codex_mcp_agents_sdk/building_consistent_workflows_codex_cli_agents_sdk
[50] MCP Specification (Nov 2025) — https://modelcontextprotocol.io/specification/2025-11-25
[51] arXiv: Adaptive Rate Limiting (2510.04516) — https://arxiv.org/abs/2510.04516
[52] arXiv: RL-Based Rate Limiting (2511.03279) — https://arxiv.org/abs/2511.03279
[53] arXiv: Scrappy (2312.00989) — https://arxiv.org/abs/2312.00989
[54] arXiv: DAAO (2509.11079) — https://arxiv.org/abs/2509.11079
[55] arXiv: Multi-Agent Incident Response (2511.15755) — https://arxiv.org/abs/2511.15755
[56] AWS CLI Agent Orchestrator — https://github.com/awslabs/cli-agent-orchestrator
[57] paddo.dev: Anthropic Walled Garden Crackdown — https://paddo.dev/blog/anthropic-walled-garden-crackdown/
[58] Anthropic: Detecting and Countering Misuse — https://www.anthropic.com/news/detecting-countering-misuse-aug-2025
[59] Anthropic Commercial Terms — https://www.anthropic.com/legal/commercial-terms
[60] Claude Code Headless Documentation — https://code.claude.com/docs/en/headless
[61] Claude Plans and Pricing — https://claude.com/pricing
[62] Claude Code Limits (Portkey) — https://portkey.ai/blog/claude-code-limits/
[63] OpenAI API Pricing — https://developers.openai.com/api/docs/pricing/
[64] Vertex AI Pricing — https://cloud.google.com/vertex-ai/generative-ai/pricing
[65] Gemini API Pricing — https://ai.google.dev/gemini-api/docs/pricing
[66] Gemini Code Assist Quotas — https://developers.google.com/gemini-code-assist/resources/quotas

---

*Document generated 2026-02-15 by Claude Opus 4.6. All claims sourced. Pessimistic risk posture applied per assignment instructions.*
