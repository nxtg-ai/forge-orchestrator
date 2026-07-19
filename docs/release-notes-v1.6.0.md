# forge-orchestrator v1.6.0 — release notes (DRAFT, staged for the batched train gate)

**Status**: staged. Version bump (`Cargo.toml` 1.5.2 → 1.6.0), tag, and GitHub release are **held**
until the batched Codex train gate PASSes (pod §1.5 final + DIRECTIVE-16 + W2-C together).

## Headline

The **portfolio consolidation** train: forge-orchestrator absorbs cosmux (`forge pod`), gains a
fleet context-budget HUD (`forge status --budget`), a unified health gate (`forge doctor`), and
closes a cross-product project-binding leak. No new MCP tools — the count stays at **11**.

## What's new

### `forge pod` — cosmux consolidation (W2-A)
Declarative tmux pod management vendored from cosmux v0.4.2, operating on the same store in place.
11 public verbs + 2 hidden recovery verbs; parity verified against the 14 live fleet pod shapes.
The §1.5 **adoption protocol** (`adopt`/`unadopt`/`--repair`/`--abort` + a standalone rollback
script) performs a single-writer cutover guarded by a transition journal whose terminal `adopted`
state is the sole production-write authority — one `flock` over all journal RMW, and every journal
parse/validate/mutate goes through one JSON parser (no grep/sed/awk shortcuts). Novel `.forge/`
synergy: a `task:`-bound pane's dead-pane recovery re-claims the forge task and logs `PaneRecovered`.

### Fleet ctx% budget HUD (W2-C)
`forge status --budget [--json] [--all]` reads each agent pane's `ctx` gauge (read-only, gauge-only)
and reports a PREP/COMPACT/STOP band per the token-budget canon. Adapter-based extraction (Claude
`ctx:NN%`, Codex `NN% left` → `100-left`, others `n/a`). A one-line dashboard strip toggles with `b`.

### `forge doctor` — unified health gate (W2-B phase 1)
Aggregates quality, release-debt, and drift into one fail-closed verdict (`--strict`, `--json`
under `forge.doctor.report.v1`). Release-debt checks lockfile agreement and multi-surface version
agreement across a repo, not a single manifest.

### Per-invocation MCP project binding (DIRECTIVE-16)
`forge mcp` now honors `FORGE_PROJECT_ROOT` and refuses `forge_set_project` under an explicit
binding, closing a cross-product contamination where one consumer's project leaked into another's.

## Platform artifacts — full 5-target matrix

v1.6.0 ships the complete matrix via `release.yml`:

| Target | Note |
|---|---|
| `x86_64-unknown-linux-musl` | **Static** (glibc-independent) — the CLX9 (glibc < 2.39) install target. Verified locally: `file` → static-pie, `ldd` → not a dynamic executable. |
| `aarch64-unknown-linux-gnu` | — |
| `x86_64-apple-darwin` | — |
| `aarch64-apple-darwin` | — |
| `x86_64-pc-windows-msvc` | — |

**v1.5.2 artifact debt retired here.** v1.5.2 was released manually during a GitHub Actions
org-wide **billing lock** (run `29665177260`: every job failed in 2–4s with "account is locked due
to a billing issue"), so only the locally-built **musl** binary was attached — the other 4 platform
artifacts were never produced. Rather than back-fill v1.5.2 (`PRM-NXTG-20260718-02`), v1.6.0's full
5-target release **supersedes** that debt: users on every platform get a complete, newer release.

## Release checklist (execute on train-gate PASS)

1. `Cargo.toml` `version = "1.6.0"` (staged; commit now).
2. Roll CHANGELOG `[Unreleased]` → `[1.6.0] - <date>`.
3. `cargo build --release` to refresh `Cargo.lock`; commit.
4. `git tag v1.6.0 && git push origin main v1.6.0`.
5. `release.yml` builds + attaches all 5 targets (requires CI billing restored — confirm green).
6. `gh release create v1.6.0 --notes-from-tag` (or from this file).
7. Verify the musl binary installs on CLX9 (glibc < 2.39).

## Gate posture

Codex weekly budget is at ~13% — the train ships under **one batched gate** (pod final + -16 +
W2-C together), not per-cure rounds. Tests: 543 green; `fmt` + `clippy -D warnings` clean; live
`~/.cosmux/state.json` md5 unchanged across the entire suite; MCP tool count 11.
