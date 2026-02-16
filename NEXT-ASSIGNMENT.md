# Assignment: DX-040 — Adaptive Token Bucket Pacing

> **Scope:** CODE CHANGES. Replace fixed pacing delays with Adaptive Token Bucket algorithm.
> **Priority:** HIGH — v1.3.0 "The Safe Operator"
> **Tests:** Must add tests for all new functionality. Target: 218+ total tests (currently 208).
> **Build:** Must compile clean (`cargo build --release`), clippy clean, `cargo fmt` applied.

## Context

DX-033 implemented fixed random delays (64-179s) between subscription-mode task dispatches. This is wasteful — too slow when no rate limits, too fast when approaching limits. RESEARCH-001 cited arXiv:2510.04516 showing Adaptive Token Bucket (ATB) achieves 97% fewer 429 errors with only 19% duration increase vs fixed delays.

**Current system (DX-033):**
- `SchedulerConfig.pacing_min_secs` (default: 64) and `pacing_max_secs` (default: 179)
- `agent_pacing: HashMap<AgentType, Instant>` in App — stores next-dispatch-allowed time
- After each subscription-mode spawn, random delay between min..max is applied
- Same fixed system in `start.rs` (headless mode)

**New system (DX-040):**
- Adaptive Token Bucket per provider
- Starts conservative, increases rate when no 429s, halves on rate limit
- Never exceeds 60% of documented quota
- Works in both dashboard (app.rs) and headless (start.rs) modes

## Data Model

### New struct in `src/tui/app.rs` (above `App`):

```rust
use std::collections::VecDeque;

/// Adaptive Token Bucket for per-provider pacing (DX-040).
/// Replaces fixed random delays with a self-adjusting rate limiter.
pub struct AdaptiveTokenBucket {
    /// Current tokens available
    pub tokens: f64,
    /// Maximum burst capacity
    pub max_tokens: f64,
    /// Current refill rate (tokens per second)
    pub refill_rate: f64,
    /// Floor refill rate (never go below this)
    pub min_refill_rate: f64,
    /// Ceiling refill rate (60% of documented quota rate)
    pub max_refill_rate: f64,
    /// Last time tokens were refilled
    pub last_refill: Instant,
    /// Count of 429s in the current sliding window
    pub window_429_count: u32,
    /// Start of current observation window
    pub window_start: Instant,
}

impl AdaptiveTokenBucket {
    /// Create a new ATB calibrated to a provider's documented quota.
    pub fn new(agent: &AgentType) -> Self {
        // Provider quotas (subscription mode):
        //   Claude: 50 tasks / 5h = 0.00278/s
        //   Codex:  60 tasks / 5h = 0.00333/s
        //   Gemini: 1000 tasks / 24h = 0.01157/s
        let quota_rate = match agent {
            AgentType::Claude => 50.0 / (5.0 * 3600.0),
            AgentType::Codex => 60.0 / (5.0 * 3600.0),
            AgentType::Gemini => 1000.0 / (24.0 * 3600.0),
            _ => 50.0 / (5.0 * 3600.0),
        };

        let max_refill = quota_rate * 0.6; // Never exceed 60% of documented quota

        Self {
            tokens: 1.0,                          // Start with 1 token (immediate first dispatch)
            max_tokens: 3.0,                       // Allow small burst
            refill_rate: max_refill * 0.5,         // Start at 50% of ceiling (conservative)
            min_refill_rate: max_refill * 0.1,     // Floor: 10% of quota
            max_refill_rate: max_refill,           // Ceiling: 60% of quota
            last_refill: Instant::now(),
            window_429_count: 0,
            window_start: Instant::now(),
        }
    }

    /// Refill tokens based on elapsed time.
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;
    }

    /// Try to consume a token. Returns true if a task can be dispatched.
    pub fn try_acquire(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Seconds until next token is available.
    pub fn seconds_until_ready(&mut self) -> u64 {
        self.refill();
        if self.tokens >= 1.0 {
            return 0;
        }
        let needed = 1.0 - self.tokens;
        (needed / self.refill_rate).ceil() as u64
    }

    /// Record a rate limit (429). Immediately halves refill rate.
    pub fn record_rate_limit(&mut self) {
        self.refill_rate = (self.refill_rate * 0.5).max(self.min_refill_rate);
        self.tokens = 0.0; // Drain all tokens
        self.window_429_count += 1;
    }

    /// Record a successful dispatch. If no recent 429s, gradually increases rate.
    pub fn record_success(&mut self) {
        // Reset window every 15 minutes
        if self.window_start.elapsed() > std::time::Duration::from_secs(15 * 60) {
            self.window_429_count = 0;
            self.window_start = Instant::now();
        }

        // Only speed up if no 429s in current window
        if self.window_429_count == 0 {
            self.refill_rate = (self.refill_rate * 1.1).min(self.max_refill_rate);
        }
    }
}
```

### Modify `App` struct

Replace `agent_pacing` with `agent_atb`:

```rust
// REMOVE this line:
// pub agent_pacing: HashMap<AgentType, Instant>,

// ADD this line:
/// Per-agent adaptive pacing (DX-040, replaces DX-033 fixed delays).
pub agent_atb: HashMap<AgentType, AdaptiveTokenBucket>,
```

In `App::new()`:
```rust
// REMOVE: agent_pacing: HashMap::new(),
// ADD:
agent_atb: HashMap::new(),
```

### Modify `schedule_unblocked_tasks` in `App`

Replace the DX-033 pacing filter (the `.filter` that checks `agent_pacing`) with ATB:

```rust
.filter(|t| {
    // DX-040: Adaptive Token Bucket pacing (replaces DX-033 fixed delays)
    let agent = t.assigned_to.clone().unwrap_or(AgentType::Claude);
    let agent = if agent == AgentType::Any { AgentType::Claude } else { agent };
    let agent_name = agent.to_string().to_lowercase();
    let auth_mode = state_mgr
        .get_agent_auth(&agent_name)
        .unwrap_or_else(|_| "subscription".to_string());
    if auth_mode == "api" {
        return true; // API mode = no pacing needed
    }
    // Check ATB
    let atb = self.agent_atb
        .entry(agent.clone())
        .or_insert_with(|| AdaptiveTokenBucket::new(&agent));
    atb.try_acquire()
})
```

Remove the DX-033 pacing cooldown code AFTER the `for task in candidates` loop (the block that inserts into `agent_pacing`). The ATB `try_acquire()` already consumed the token.

### Modify `handle_agent_event` — success path

After `self.agent_backoff.remove(&agent)` on success, add:

```rust
// DX-040: Record success in ATB for adaptive rate adjustment
if let Some(atb) = self.agent_atb.get_mut(&agent) {
    atb.record_success();
}
```

### Modify `handle_agent_event` — rate limit path

In the rate-limited branch, after `backoff.attempt += 1`, add:

```rust
// DX-040: Record rate limit in ATB — halves refill rate
if let Some(atb) = self.agent_atb.get_mut(&agent) {
    atb.record_rate_limit();
}
```

### Dashboard footer: Show ATB status

In `build_quota_spans` in `src/tui/ui.rs`, for subscription mode, change the format from `(5h)` to show pacing info:

For subscription mode, after the color-coded count, show the ATB wait time. Change:
```rust
format!("{}: {}/{} (5h)", agent_type, count, max)
```
To:
```rust
format!("{}: {}/{}", agent_type, count, max)
```

(Just remove the `(5h)` — the ATB pacing is implicit and doesn't have a fixed window to display.)

## Headless mode (`src/cli/start.rs`)

### Add ATB to `ProviderState`

Modify `ProviderState`:

```rust
#[derive(Debug)]
struct ProviderState {
    consecutive_rate_limits: u32,
    paused_until: Option<Instant>,
    /// DX-040: Adaptive Token Bucket for pacing
    atb: Option<AdaptiveTokenBucket>,
}

impl Default for ProviderState {
    fn default() -> Self {
        Self {
            consecutive_rate_limits: 0,
            paused_until: None,
            atb: None,
        }
    }
}
```

Import `AdaptiveTokenBucket` from `crate::tui::app::AdaptiveTokenBucket` in `start.rs`.

### Replace fixed pacing in `run_agent_loop`

Replace the DX-033 subscription pacing block at the end of the loop:

```rust
// REMOVE this block:
// if success && auth_mode == "subscription" {
//     let delay = rand::rng().random_range(scheduler.pacing_min_secs..=scheduler.pacing_max_secs);
//     println!("  {tag} {} Subscription pacing: waiting {}s...", "⏳".dimmed(), delay);
//     tokio::time::sleep(Duration::from_secs(delay)).await;
// }

// ADD this block:
// DX-040: Adaptive Token Bucket pacing (replaces DX-033 fixed delays)
if success && auth_mode == "subscription" {
    let mut tracker = provider_tracker.lock().await;
    let pstate = tracker.entry(agent.clone()).or_default();
    let atb = pstate.atb.get_or_insert_with(|| {
        crate::tui::app::AdaptiveTokenBucket::new(agent)
    });
    atb.record_success();
    let wait = atb.seconds_until_ready();
    if wait > 0 {
        println!(
            "  {tag} {} Adaptive pacing: waiting {}s (rate: {:.4}/s)",
            "⏳".dimmed(),
            wait,
            atb.refill_rate,
        );
        drop(tracker);
        tokio::time::sleep(Duration::from_secs(wait)).await;
    }
}
```

Also, where rate limits are recorded (the `hit_rate_limit` block), add ATB recording:

```rust
// After pstate.consecutive_rate_limits += 1;
let atb = pstate.atb.get_or_insert_with(|| {
    crate::tui::app::AdaptiveTokenBucket::new(agent)
});
atb.record_rate_limit();
```

### Update pacing info display

In the header section of `execute()` that shows pacing info, update the subscription message:

```rust
// CHANGE:
// "pacing {}-{}s between tasks"
// TO:
"adaptive pacing (ATB, 60% quota ceiling)"
```

## SchedulerConfig changes (`src/core/state.rs`)

Keep `pacing_min_secs` and `pacing_max_secs` for backward compatibility (old state.json files). They are no longer used by the ATB algorithm but won't break deserialization.

Add to `SchedulerConfig`:

```rust
/// Pacing strategy: "adaptive" (ATB, default) or "fixed" (legacy DX-033 random delays).
#[serde(default = "default_pacing_strategy")]
pub pacing_strategy: String,
```

Add helper:
```rust
fn default_pacing_strategy() -> String {
    "adaptive".to_string()
}
```

Update `SchedulerConfig::default()`:
```rust
impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            rotation: false,
            pacing_min_secs: 64,
            pacing_max_secs: 179,
            pacing_strategy: "adaptive".to_string(),
        }
    }
}
```

In both `app.rs` and `start.rs`, check `scheduler.pacing_strategy` — if `"fixed"`, use the old DX-033 behavior. If `"adaptive"` (default), use ATB.

## `forge config` support

In `src/cli/config.rs`, add handling for `scheduler.pacing`:

```rust
"scheduler.pacing" => {
    if let Some(val) = value {
        match val.as_str() {
            "adaptive" | "fixed" => {
                let mut state = state_mgr.load()?;
                state.scheduler.pacing_strategy = val.clone();
                state_mgr.save(&state)?;
                println!("  → Pacing strategy: {}", val);
            }
            _ => println!("  ✗ Invalid value. Use 'adaptive' or 'fixed'."),
        }
    } else {
        let state = state_mgr.load()?;
        println!("  scheduler.pacing = {}", state.scheduler.pacing_strategy);
    }
}
```

## Tests

Add to `src/tui/app.rs` tests:

1. `test_atb_new_has_initial_token` — verify new ATB starts with 1 token and can dispatch immediately
2. `test_atb_try_acquire_consumes_token` — verify token consumed, second acquire fails
3. `test_atb_refill_over_time` — create ATB, consume token, verify tokens increase after simulated time (modify `last_refill` to past)
4. `test_atb_record_rate_limit_halves_rate` — verify refill_rate halves on 429, but not below min
5. `test_atb_record_success_increases_rate` — verify rate increases by 10% when no 429s in window
6. `test_atb_rate_never_exceeds_ceiling` — record many successes, verify rate caps at max_refill_rate
7. `test_atb_rate_never_below_floor` — record many 429s, verify rate stops at min_refill_rate
8. `test_atb_seconds_until_ready` — verify time calculation is accurate

Add to `src/core/state.rs` tests:

9. `test_scheduler_config_default_adaptive` — verify default pacing_strategy is "adaptive"
10. `test_scheduler_config_backward_compat` — verify old JSON without pacing_strategy deserializes with default "adaptive"

## File Summary

| File | Changes |
|------|---------|
| `src/tui/app.rs` | Add `AdaptiveTokenBucket` struct (public), replace `agent_pacing` with `agent_atb`, update `schedule_unblocked_tasks` filter, record success/429 in event handler, add 8 ATB tests |
| `src/cli/start.rs` | Add ATB to `ProviderState`, replace fixed pacing with ATB-based wait, record 429s in ATB, update header display, import ATB |
| `src/tui/ui.rs` | Remove `(5h)` from quota display format |
| `src/core/state.rs` | Add `pacing_strategy` field to `SchedulerConfig`, add 2 tests |
| `src/cli/config.rs` | Add `scheduler.pacing` config key handler |

## Build & Test

```bash
cargo fmt
cargo clippy -- -W clippy::all
cargo test
cargo build --release
cp target/release/forge ~/.local/bin/forge-orca
```

All 218+ tests must pass. Zero clippy warnings. Binary deploys.

## IMPORTANT NOTES

- Make `AdaptiveTokenBucket` and its fields `pub` so `start.rs` can import and use it
- Keep `pacing_min_secs` and `pacing_max_secs` in `SchedulerConfig` for backward compat — just don't use them when strategy is "adaptive"
- The old `agent_pacing: HashMap<AgentType, Instant>` field is REMOVED entirely — replaced by `agent_atb`
- If `scheduler.pacing_strategy` is "fixed", fall back to old DX-033 behavior using `agent_pacing` logic (re-implement locally, don't keep the field)
- `cargo fmt` MUST be the last step before build

---

**CHECKPOINT: 218+ tests, clippy clean, cargo fmt applied, binary deployed.**
