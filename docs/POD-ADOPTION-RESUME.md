# RESUME FIRST — `forge pod` §1.5 Adoption

**Banked**: 2026-07-18 22:30 PDT · **Reason**: pane at ~516k tokens, past the 500k COMPACT
threshold, at an atomic boundary (checkpoint 3 shipped and pushed).
**Repo**: `/home/axw/projects/NXTG-Forge/forge-orchestrator` · **Branch**: `main` · **Tip**: `2b025cb`
**Directive**: DIRECTIVE-NXTG-20260718-09 (`/home/axw/projects/NXTG-Forge/.asif/NEXUS.md`)
**Spec**: `/home/axw/projects/NXTG-Forge/ecosystem/forge/research/2026-07-18-consolidation-rfc.md`
(v10.1, GATED-PASS, Codex round 10, zero blocking findings)

---

## 0. THE CONSTRAINT — read this before writing any code

**Never touch the live `~/.cosmux/state.json` or live tmux sessions in tests.**

That store tracks **running fleet pods** (e.g. `Dx3_Program`, started 2026-04-19). There were
**32 live tmux sessions** at bank time. Corrupting either breaks fleet pod tracking for every lane.
This is the only irreversible surface in this directive.

Two seams already exist and are proven — **use them, do not re-invent them**:

| Env var | Default (production, unchanged) | Purpose |
|---|---|---|
| `FORGE_POD_STATE_DIR` | `~/.cosmux` | Redirect the store. Under `cfg(test)` a write outside the temp dir **panics** — an override alone is insufficient when the default is destructive. |
| `FORGE_POD_TMUX_SOCKET` | *(unset → default server)* | `tmux -L <socket>`. Tests point at a private/nonexistent socket, so the live server is unreachable **by construction**, not by care. |
| `FORGE_POD_TEMPLATE_DIR` | `~/.config/cosmux/templates` | Redirect template lookup. |

**Standing instrument, re-run before AND after every test run:**

```bash
md5sum ~/.cosmux/state.json     # baseline: 3b613cf8366d41a445026d55d00c433c
tmux ls | wc -l                 # baseline: 32 sessions
```

Both were unchanged across all of checkpoints 1–3. Keep it that way; report the md5 in the
Response as the constraint's proof.

---

## 1. WHERE THINGS STAND — checkpoint 3 COMPLETE

`forge pod` core is done and pushed. **-09 items 1–6 complete for the core**; only the migration
surface remains (this document).

| Item | State |
|---|---|
| 1 — vendor cosmux core | **DONE** — `error` · `config` · `state` · `templates` · `tmux` · `hooks` · `recover` · `preflight` |
| 2 — CLI + parity matrix | **DONE** — 11 public verbs + 2 hidden; `docs/pod-parity-matrix.md` |
| 3 — single-store in place | **DONE** — `~/.cosmux/state.json` + legacy search order verbatim |
| 4 — `.forge/` synergy | **DONE** — `task:` binding + `PaneRecovered` event |
| 5 — tests | **DONE** — 14-shape dual-run parity, `tests/pod_parity.rs` |
| 6 — plan first | **DONE** |
| **§1.5 adoption** | **NOT STARTED — this is the work** |

### Commit trail (this session, all pushed to `origin/main`)

| SHA | What |
|---|---|
| `2b025cb` | pod checkpoint 3 — preflight, 11-verb CLI, parity matrix, 14-shape dual-run |
| `a1019cf` | pod checkpoint 2 — tmux seam, hooks, recovery, `PaneRecovered` |
| `448d542` | pod checkpoint 1 — config/state/templates + the fail-closed store seam |
| `f270ae7` | doctor D2 — stop swallowing unreadable surfaces / marketplace entries |
| `56763f9` | doctor D1 — repo-aware surface inventory (nested + marketplace) |
| `44668de` | `forge doctor` + release-debt gate |
| `a1af359` | RFC-0001 rev 4 — malformed records skipped, never fatal |
| `2b82e56` | RFC-0001 rev 3 — per-record version comparison + batch semantics |
| `5170b00` | RFC-0001 rev 2 — withdrew false G-04 premise, baseline `2.0.0`→`1.1.0` |
| `5c412c2` | v1.5.2 release train |

**DON'T REDO**: everything above is on `origin/main`. Diff against HEAD before rebuilding anything.

### Current gate state

- `cargo test` → **478 passed / 0 failed**. Never let this decrease.
- `cargo fmt --check` clean · `cargo clippy -- -D warnings` clean (CI's invocation).
  Note `--all-targets` has **14 pre-existing** errors on clean HEAD in older test code — verified
  unchanged by stashing; not mine, flagged to FPL, not in scope.
- MCP tool count still **11** (`src/mcp/tools.rs:793`). Must not change.
- `Cargo.toml` is **1.5.2** — **do NOT bump**. This rides the v1.6.0 train; the pre-push hook
  blocks a bump without tag + dated CHANGELOG.
- `forge doctor --strict` on this repo exits **1** with `unreleased-commits: N commits since
  v1.5.2`. That is a **true finding**, not a regression — it discharges when v1.6.0 ships. Do not
  tune the threshold to make it green.

---

## 2. THE §1.5 PUNCH LIST — implement exactly as gated

The HOLD on adopt/shim/hook-rebind was **LIFTED** by FPL after Codex round 10 cleared the
consolidation RFC with zero blocking findings. Implement §1.5 **exactly as specified** — this was
gated on the precise mechanics below, so improvising a "better" design re-opens the gate.

### Order of work — non-negotiable

> **BUILD THE ISOLATION AND THE RACE MATRIX BEFORE ANYTHING THAT ADOPTS.**
> Adoption mutates live session state. The test harness that proves it cannot touch the fleet must
> exist *before* the first line of adopt logic, not after.

### 2.1 Transition journal

- **Path**: `~/.local/state/forge/pod-adoption.json`
- **Schema v1**, and it **must include the `recoveries` step** (the step list is part of the gated
  spec — read §1.5 for the exact step set and field names; do not infer them from this summary).

### 2.2 Locking — single `flock`, no exceptions

- **One `flock` on `pod-adoption.lock` guards ALL journal read-modify-write**, including the
  **terminal re-read-under-lock**. That terminal re-read is the part most likely to be dropped by
  accident; it is explicitly in scope.
- Every mutation path takes the same lock. No second lock, no lock-free "fast path" read that then
  writes.

### 2.3 Standalone unadopt script

- Standalone (not a `forge` subcommand), and it **honours the same lock**.
- Sequence: **`flock` + `rm` + `tmux`**.
- It must work when the `forge` binary is unavailable — that is the point of it being standalone.

### 2.4 Failure-injection + race acceptance matrix

- Build it **as specified in §1.5** — the matrix shape is gated, not free-form.
- Failure injection at each journal step; races between concurrent adopt/unadopt.

### 2.5 Read the spec, don't work from this summary

This file is a **resume pointer, not the specification**. Before writing code, read
`ecosystem/forge/research/2026-07-18-consolidation-rfc.md` **§1.5** in full. Where this summary and
the RFC differ, **the RFC wins** — it is what Codex gated.

---

## 3. FIRST ACTIONS ON RESUME

1. `date` + `git log -1 --oneline` + `md5sum ~/.cosmux/state.json` + `tmux ls | wc -l` — establish
   current truth before claiming anything.
2. Read consolidation RFC **§1.5** in full.
3. Read `docs/pod-parity-matrix.md` §"Not implemented (gated migration surface)" — the boundary
   between what shipped and what §1.5 adds.
4. Build the **isolation harness + race matrix first**.
5. Then adopt / shim / hook-rebind.
6. Write the Response inline in `/home/axw/projects/NXTG-Forge/.asif/NEXUS.md` under
   DIRECTIVE-NXTG-20260718-09 with real SHAs and the live-store md5 as the constraint proof.

## 4. Working norms that have been earning their keep

- **Reproduce before fixing.** Every Codex finding this session was reproduced first; twice that
  changed the diagnosis.
- **Self-probe the neighbours.** Codex's round-3 note ("attacks adjacent invariants") found 3 extra
  bugs of the same class it had not reported. Do this before signalling ready.
- **Negative-control every fix.** Re-introduce the defect, watch the new test fail, restore. A test
  that cannot fail proves nothing.
- **Re-probe before reporting a live finding.** A mid-release read produced a "live desync in
  forge-plugin" that was transient and would have been a false report.
- **Correct the directive when instruments disagree with it** (dep delta was +1, not +3; `hud` is
  an alias; the plugin has 5 surfaces, not 4). FPL has accepted every such correction.
