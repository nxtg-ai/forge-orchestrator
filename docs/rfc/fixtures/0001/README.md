# RFC-0001 Compatibility Fixtures

Executable fixtures for [RFC-0001 — Schema-Version Handshake for `.forge/` Cross-Repo Artifacts](../../0001-forge-artifact-schema-versioning.md).

**Status: proposal-stage.** Nothing in `forge-orchestrator/src/` reads or writes a schema version yet.
These fixtures exist so forge-orchestrator, forge-ui, and forge-plugin can agree on the same behavior
*before* any of the three writes reader code — and so the version rules cannot silently drift out of
agreement with the RFC prose.

## Run

```bash
python3 check_fixtures.py       # 9 cases; exit 0 = conform
python3 check_fixtures.py -v    # also print why each case exists
```

Stdlib only, no dependencies, no repo coupling. Current state: **9/9 passing.**

## What is here

| File | Role |
|---|---|
| `expectations.json` | The specification, machine-readable. Each case is a `(supported_version, fixture, expected_verdict)` triple plus expected parse counts. Edit this to change the contract. |
| `check_fixtures.py` | The executable form of RFC-0001 §3.2.1's comparison rule. Asserts every case. |
| `state/*.json` | `.forge/state.json` fixtures |
| `events/*.jsonl` | `.forge/events.jsonl` fixtures |

## The matrix

Both compatibility directions are covered, because they fail differently. "Downgrade" — an older reader
meeting a newer file — is the case the rule exists for, and it splits: MINOR-ahead must keep working,
MAJOR-ahead must refuse.

| Case | File | Reader | Verdict |
|---|---|---|---|
| `state-absent-field-accepted-as-1.0.0` | absent ⇒ 1.0.0 | 1.1.0 | ACCEPT |
| `state-baseline-exact-match` | 1.1.0 | 1.1.0 | ACCEPT |
| `state-minor-ahead-downgrade-tolerated` | 1.2.0 | 1.1.0 | ACCEPT_FORWARD |
| `state-major-ahead-refused` | 2.0.0 | 1.1.0 | REFUSE (names both versions) |
| `state-file-older-major-refused` | 1.1.0 | 2.0.0 | REFUSE |
| `events-absent-field-accepted-as-1.0.0` | absent ⇒ 1.0.0 | 1.1.0 | ACCEPT, 3 parsed / 0 skipped |
| `events-unknown-variant-not-fatal` | 1.2.0 | 1.1.0 | ACCEPT_FORWARD, 3 parsed / 0 skipped |
| `events-corrupt-line-skipped-and-counted` | 1.1.0 | 1.1.0 | ACCEPT, 3 parsed / 1 skipped |
| `events-major-ahead-refused` | 2.0.0 | 1.1.0 | REFUSE |

The two highest-value rows:

- **`events-unknown-variant-not-fatal`** carries an `event_type` of `provenance_recorded`, which is not
  one of the 14 known variants. This is the actual G-05 hazard: without `#[serde(other)]` on `EventType`,
  a single record like this errors the whole batch and turns an older dashboard blank. The fixture is what
  a *future* forge release's log looks like to *today's* reader.
- **`events-corrupt-line-skipped-and-counted`** has a truncated line 2. One bad line must not zero out a
  10,000-line history.

## Baseline rule (guarded)

`absent ⇒ 1.0.0`, first versioned release ⇒ `1.1.0`. No file ever literally declares `1.0.0` — a reader
infers it from the field's absence. Adding the version field is itself an optional-field addition, which
is MINOR by the RFC's own change table, hence `1.1.0`.

`check_fixtures.py` asserts this relationship before running any case and exits `2` if
`expectations.json` violates it. That guard exists because an earlier RFC draft started the scheme at
`2.0.0` — manufacturing a MAJOR boundary, and a mandatory refusal, with no breaking shape change (caught
by Codex Wave-1 review). The guard makes that specific regression mechanically impossible to reintroduce.

## Consuming from another repo

The fixtures are plain data; the checker is a reference implementation, not a dependency.

- **Rust (forge-orchestrator)** — table-drive `expectations.json` in a `#[test]`; `serde_json` reads the
  fixtures directly. Verify unknown variants deserialize to the `#[serde(other)]` catch-all rather than
  erroring.
- **JS (forge-ui, governance-mcp)** — `JSON.parse` tolerates unknown fields and unknown enum *values*
  natively, so the risk is different: do not `switch` on `event_type` without a `default` branch, and do
  not assume `state_schema` exists. Drive the same matrix in vitest/node:test.

When a repo implements the handshake, it should add its own runner over this same `expectations.json`
rather than copying the cases — one spec, three readers.
