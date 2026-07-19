# RFC-0001 — Schema-Version Handshake for `.forge/` Cross-Repo Artifacts

**Status**: PROPOSED (no implementation in `src/`)
**Author**: forge-orchestrator team
**Date**: 2026-07-18 | **Revised**: 2026-07-18 (rev 4)
**Origin**: DIRECTIVE-NXTG-20260718-03 items 3 & 4 — deep-dive gaps G-04, G-05
**Rev 2** (DIRECTIVE-NXTG-20260718-06, Codex Wave-1 review
`ecosystem/forge/research/2026-07-18-codex-wave1-review.md`): withdrew §4's false collision
premise (verified against forge-plugin `v3.10.2`) and corrected the schema baseline from
`2.0.0` to `1.1.0`, pinned by executable fixtures (§3.5).
**Rev 3** (Codex re-gate round 2, `ecosystem/forge/research/2026-07-18-codex-regate-2.md` §3):
added §3.2.2 — per-record comparison and batch semantics for mixed-version event logs. Rev 2's
checker derived one version from the first record, so a `1.1.0` reader silently accepted a
`2.0.0` record appended later. Three mixed-order fixtures now pin it.
**Rev 4** (Codex re-gate round 3, `…/2026-07-18-codex-regate-3.md` §3): added §3.2.3 — malformed
records are skipped, never fatal. A structurally-invalid but JSON-valid line (`[]`) crashed the whole
batch. Self-probing adjacent invariants found three more of the same class Codex had not reported
(non-string `v`, non-numeric `v`, and the identical crash on the `state.json` path); all four are
fixtures, and `state.json` gained a `MALFORMED` verdict.
**Consumers affected**: forge-ui, forge-plugin (governance-mcp), forge-orchestrator
**Decision needed from**: FPL (circulate), forge-ui team, forge-plugin team

---

## 1. Problem

`.forge/state.json` and `.forge/events.jsonl` are the *de facto* integration contract between three
independently versioned repos. They are read by consumers the orchestrator does not control and cannot
upgrade in lockstep. Today that contract has no negotiated version, so a writer-side change is
indistinguishable from corruption at the reader.

Three concrete defects grounded in the current tree:

**D-1 — `state.json` carries a version field that nobody sets consistently.**
`ForgeState.version: String` exists (`src/core/state.rs:15`) and defaults to `"1.1.0"`
(`src/core/state.rs:151`), but `src/cli/init.rs:230` writes a JSON literal `"version": "1.0.0"` into a
separate artifact. Two version strings, no documented meaning for either, no consumer that reads them.
A field that is written but never read is not a contract — it is decoration.

**D-2 — `events.jsonl` has no version at all.**
`ForgeEvent` (`src/core/event.rs:28-40`) has no version field, and the file is append-only, so a single
file can contain records written by several orchestrator versions across its lifetime. There is no
per-record way for a reader to know which shape it is looking at. This is the higher-risk artifact of
the two: `state.json` is rewritten whole, `events.jsonl` is *historical* and inherently mixed-version.

**D-3 — a new `EventType` variant is a hard break for every reader (the actual G-05 hazard).**
`EventType` (`src/core/event.rs:8-24`) is a plain externally-tagged serde enum with 14 variants. Serde's
default behavior for an unknown variant is a hard deserialization error. So the *next* orchestrator
release that adds, say, `ProvenanceRecorded` silently converts every older consumer's event-log read
from "returns 500 events" to "errors on the whole batch" the first time that variant appears — including
`forge_get_events`, the dashboard event feed, and any governance scorer that tails the log. Adding an
event type is currently a **breaking change disguised as an additive one**.

Optional-field additions (`duration_ms`, `exit_code`, `current_phase`) are already handled correctly via
`#[serde(default, skip_serializing_if = "Option::is_none")]` — that pattern is sound and this RFC keeps
it. The gap is variants, not fields.

---

## 2. Non-goals

- No wire-format change in v1.5.2. This RFC is a proposal; implementation is a follow-up wave.
- No new MCP tools (directive constraint — the 11-tool count stands).
- No migration of existing `.forge/` directories in the field. Whatever ships must read today's
  unversioned files without complaint.

---

## 3. Proposal

### 3.1 Versioning scheme

Both artifacts carry a semver **schema** version, versioned independently of the `forge` binary version.
They describe file shape, not product releases; coupling them to the binary version forces meaningless
churn (a CLI-only release must not bump a schema nobody changed).

- `state.json` → `state_schema` at the document root.
- `events.jsonl` → `v` on **each record** (short key: this is a hot append path with one write per
  event; the field is repeated on every line).

Semantics:

| Change | Bump | Reader obligation |
|---|---|---|
| New optional field | MINOR | Ignore unknown fields (already true) |
| New enum variant (e.g. `EventType`) | MINOR | Must not error — see 3.2 |
| Field removed / retyped / semantics changed | MAJOR | Reader may refuse and must say so |
| Value-only change (new `dashboard_mode` string) | PATCH | None |

**Baseline: absent ⇒ `1.0.0`; first versioned release ⇒ `1.1.0`.**

`1.0.0` is the *implied* shape of every `.forge/` artifact written up to and including forge v1.5.2 —
the shape that exists today, before this RFC. No file ever literally declares `1.0.0`: a reader infers
it from the field's absence. The first release that implements this RFC adds one optional root field
(`state_schema`) and one optional per-record field (`v`), which by the table above is a **MINOR** change,
so that release declares `1.1.0`.

An earlier draft of this RFC started the scheme at `2.0.0`. That was wrong and is corrected here: it
manufactured a MAJOR boundary — and therefore a mandatory refusal under §3.2 rule 2 — without any
breaking shape change, which would have made every pre-RFC artifact unreadable to a compliant new reader
for no reason. The motivation was to avoid colliding with the legacy `ForgeState.version` values
(`1.0.0`/`1.1.0`), but those live under a **different key** (`version`, not `state_schema`), so there was
never a collision to avoid. Codex Wave-1 review caught this; the fixtures in §3.5 now pin the rule so the
inconsistency cannot silently return.

The existing `ForgeState.version` field is **deprecated, not repurposed** — it is ambiguous in the field
(D-1). New readers use `state_schema` and ignore `version` entirely; the old field is written unchanged
for one minor cycle, then dropped in the next MAJOR.

### 3.2 Compatibility rule for consumers

The rule is asymmetric on purpose, because the failure modes are asymmetric:

1. **Absent version ⇒ treat as `1.0.0`.** Every `.forge/` directory in the field today is version-less.
   A reader that requires the field breaks every existing project on upgrade.
2. **MAJOR mismatch (reader < writer) ⇒ refuse, loudly, with both versions named.** Degrading silently
   on a shape change is the silent-swallow class this repo just spent Gate 5 removing.
3. **MINOR/PATCH ahead of the reader ⇒ proceed.** Unknown fields ignored; unknown enum variants mapped
   to a catch-all, never fatal.
4. **Never fail the whole batch on one bad record.** `events.jsonl` readers skip-and-count malformed
   lines and report the count; one unparseable line must not zero out a 10,000-line history.

The enabling change for rule 3 on the Rust side is `#[serde(other)]`:

```rust
pub enum EventType {
    // … existing 14 variants …
    /// Forward-compatibility sink: a variant written by a newer forge.
    #[serde(other)]
    Unknown,
}
```

`#[serde(other)]` only applies to unit variants and only on deserialization — it costs nothing on the
write path and makes every future variant addition genuinely additive. **This is the single highest-value
line in the RFC and the one item worth implementing first, independent of the version fields**, because
it is the difference between "old dashboards ignore a new event" and "old dashboards go blank." Note the
catch-all is lossy (the original variant string is not preserved); if consumers need the raw tag, the
alternative is `#[serde(untagged)]` on a wrapper carrying `Known(EventType)` / `Unknown(String)` — more
faithful, more invasive, deferred to implementation review.

JS consumers (forge-ui, governance-mcp) are structurally safer here — `JSON.parse` does not reject unknown
string values — but they must not *switch* on event type without a default branch, and must not assume
`state_schema` exists.

### 3.2.1 Supported-version comparison (normative)

Every reader declares the schema version it was built against — its **supported version** — and compares
it to the **file version** (absent ⇒ `1.0.0`). Only the two leading components decide the verdict; PATCH
never affects compatibility.

```
verdict(supported S, file F):
  if F.major > S.major   -> REFUSE   # downgrade case: file newer in a breaking way
  if F.major < S.major   -> REFUSE   # reader newer across a break; shape it expects is gone
  if F.minor > S.minor   -> ACCEPT_FORWARD   # file ahead within major: unknown fields/variants tolerated
  otherwise              -> ACCEPT
```

- `REFUSE` is loud and names both versions: `state.json is schema 2.0.0, this reader supports 1.1.0`.
  It never silently returns empty or zeroed data — that is the silent-swallow class Gate 5 removed.
- `ACCEPT_FORWARD` is a normal read. Unknown fields are ignored; unknown enum variants map to the
  catch-all of §3.2 rule 3. A reader MAY log once that the file is ahead; it MUST NOT warn per record.
- **Downgrade** — an older reader meeting a newer file — is the case this rule exists for, and it splits:
  MINOR-ahead must keep working (that is the whole point of the additive discipline), MAJOR-ahead must
  refuse. A reader that treats all "ahead" as fatal breaks every rolling upgrade; one that treats all
  "ahead" as fine silently misreads a changed shape.
- `F.major < S.major` also refuses: a reader built for a later major cannot assume the older shape is a
  subset. It is a distinct message (`file is older major`) and is the only case where a migration step,
  not a refusal, may later be specified.

### 3.2.2 Batch semantics for mixed-version event logs (normative)

`state.json` is rewritten whole and therefore has exactly one version. **`events.jsonl` does not.** It is
append-only, so one file routinely holds records written by several forge releases (§1 D-2) — that is
precisely why `v` is per-record and not a file header.

It follows that **the version comparison of §3.2.1 runs per record, not once per file.** Deriving one
version from the first record and applying it to the whole log silently trusts a major-ahead record
appended after compatible history — the exact hazard the per-record design exists to prevent.

For each parseable record, with `F` = that record's `v` (absent ⇒ `1.0.0`):

| Record verdict | Record disposition |
|---|---|
| `ACCEPT` / `ACCEPT_FORWARD` | Parsed into the result set; counted in `parsed` |
| `REFUSE` | **Quarantined** — never parsed, counted in `refused`, its version recorded |
| unparseable line | Counted in `skipped` (§3.2 rule 4) |
| **structurally malformed record** | Counted in `skipped` — see §3.2.3 |

### 3.2.3 Malformed records are skipped, never fatal (normative)

"One bad record must not break the batch" (§3.2 rule 4) is stated in terms of *unparseable lines*,
which is too narrow. A line can be **syntactically valid JSON and still not be a record**. Before a
reader touches any field it must establish that the line is usable:

1. **The decoded value MUST be a JSON object.** `[]`, `"a string"`, `123`, and `null` all decode
   without error and none of them is a record. A reader that goes straight to field access on the
   decoded value raises on the first such line and loses the whole log.
2. **`v`, when present, MUST be a valid three-part numeric semver string.** A non-string (`{"v": 2}`),
   a non-numeric part (`"1.x.0"`), or the wrong arity (`"1.1"`) are all malformed.

A record failing either check is counted in `skipped` — **not** `refused`. The distinction is
meaningful: `refused` says *this record is readable but newer than me, upgrade the reader*, while
`skipped` says *this record is damaged*. Reporting a malformed record as `refused` would send an
operator chasing a version-compatibility problem that does not exist.

The same rule governs `state.json`, which is a single document rather than a stream: a state file
that is not a JSON object, or whose `state_schema` is not a valid semver string, is reported as
**`MALFORMED`**. That verdict is distinct from `REFUSE` — `REFUSE` means *readable but too new*,
`MALFORMED` means *not readable at all* — and, like every other case here, it is a returned verdict
rather than a raised exception.

**The general rule: version comparison operates on validated input.** Any value read from a file is
untrusted, so a reader needs a *total* version parser — one that returns "not a version" instead of
raising — and must treat structural validation as part of reading, not as an assumption.

Origin: Codex re-gate round 3 (`ecosystem/forge/research/2026-07-18-codex-regate-3.md` §3) caught
the non-object case. Self-probing the adjacent invariants before signalling ready surfaced three
more of the same class that were not reported: non-string `v`, non-numeric `v`, and the identical
crash on the `state.json` path. All four are now fixtures.

The batch verdict is then:

| Condition | Batch verdict |
|---|---|
| `refused > 0` and `parsed == 0` | `REFUSE` — nothing in the file is readable by this reader |
| `refused > 0` and `parsed > 0` | `ACCEPT_PARTIAL` — compatible history readable, incompatible records counted |
| `refused == 0`, any record minor-ahead | `ACCEPT_FORWARD` |
| otherwise | `ACCEPT` |

`ACCEPT_PARTIAL` exists because the two obvious alternatives are both wrong. Refusing the whole log
because one future record appeared would zero a history — the failure §3.2 rule 4 forbids. Accepting it
silently is the round-2 defect. So the reader returns what it can prove it understands and **surfaces the
count and the offending version(s) loudly**; a caller that treats `refused > 0` as fatal may do so, but
that is the caller's policy, not the reader's silent choice.

Two consequences worth stating explicitly, because both are easy to get wrong:

- **Position must not determine the verdict.** A baseline record appearing *after* a major-ahead one is
  still readable. A reader that stops at the first incompatible record loses valid history behind it.
- **A quarantined record is not a corrupt record.** They are counted separately (`refused` vs `skipped`)
  because they mean different things operationally: `skipped` is data damage, `refused` is a reader that
  is behind the writer and may simply need upgrading.

Origin: Codex re-gate round 2 (`ecosystem/forge/research/2026-07-18-codex-regate-2.md` §3) — rev 2's
checker read only the first record's version, so a `1.1.0` reader silently accepted a `2.0.0` record. The
mixed-order fixtures in §3.5 now pin all of the above.

### 3.3 Rollout

| Phase | Orchestrator | Consumers |
|---|---|---|
| P0 (next minor) | Add `#[serde(other)] Unknown`; write `state_schema: "1.1.0"` + per-record `v: "1.1.0"`; keep writing legacy `version` | No change required; existing readers keep working |
| P1 | — | forge-ui + governance-mcp implement the 4 rules; add a test that a synthetic future variant does not break the reader |
| P2 (next MAJOR) | Drop legacy `ForgeState.version`; reconcile `init.rs:230` | Readers already on `state_schema` |

Consumers are never blocked on the orchestrator, and the orchestrator is never blocked on consumers.

### 3.4 Test obligations

Non-negotiable at implementation time (test count never decreases):

- Round-trip an event carrying an unknown `event_type` → deserializes as `Unknown`, does not error.
- Read a version-less `state.json` and a version-less `events.jsonl` → both accepted as `1.0.0`.
- Read a `state.json` with a MAJOR ahead of the reader → refused with an error naming both versions.
- Parse an `events.jsonl` with one corrupt line among valid ones → valid records returned, skip counted.

Each of these is already pinned as data in §3.5 — the implementing repo drives `expectations.json`
rather than hand-writing the cases.

### 3.5 Executable compatibility fixtures

The rules above are pinned by data, not prose, so all three repos can verify the same behavior:

```
docs/rfc/fixtures/0001/
  README.md                     # how to run + how to consume from Rust/JS
  expectations.json             # the (reader × file) verdict matrix — the spec
  state/*.json                  # state.json fixtures, incl. the absent-field baseline
  events/*.jsonl                # events.jsonl fixtures, incl. unknown variant + corrupt line
  check_fixtures.py             # standalone conformance checker (no repo deps)
```

`expectations.json` is the machine-readable specification: each row is a `(supported_version,
fixture, expected_verdict)` triple plus, for event logs, the expected `parsed`/`skipped` counts.
`check_fixtures.py` implements §3.2.1 and asserts every row — so the checker is the *executable form of
the comparison rule*, deliberately not an implementation of the handshake in `src/`. Nothing in
`forge-orchestrator/src/` changes until this RFC is ratified; the fixtures let forge-ui and forge-plugin
build their readers against the same matrix in parallel, and let any repo table-drive it in CI later.

```bash
python3 docs/rfc/fixtures/0001/check_fixtures.py    # 16/16 passing as of rev 4
```

The checker also refuses to run if `expectations.json` violates the §3.1 baseline rule (exit `2`), which
is what makes the `2.0.0` regression mechanically impossible to reintroduce rather than merely documented.

Coverage — all four directions the rule distinguishes, the two parse-robustness cases, and the three
mixed-version orderings (§3.2.2):

| Fixture | File version | Reader | Expected |
|---|---|---|---|
| `state/v1_0_0_absent_field.json` | absent ⇒ 1.0.0 | 1.1.0 | ACCEPT |
| `state/v1_1_0_baseline.json` | 1.1.0 | 1.1.0 | ACCEPT |
| `state/v1_2_0_minor_ahead.json` | 1.2.0 | 1.1.0 | ACCEPT_FORWARD (unknown field ignored) |
| `state/v2_0_0_major_ahead.json` | 2.0.0 | 1.1.0 | REFUSE (both versions named) |
| `state/v1_1_0_baseline.json` | 1.1.0 | 2.0.0 | REFUSE (file older major) |
| `events/v1_0_0_absent_field.jsonl` | absent ⇒ 1.0.0 | 1.1.0 | ACCEPT, 3 parsed / 0 skipped / 0 refused |
| `events/v1_2_0_unknown_variant.jsonl` | 1.2.0 | 1.1.0 | ACCEPT_FORWARD, 3 parsed — unknown `event_type` maps to the catch-all, never fatal |
| `events/v1_1_0_corrupt_line.jsonl` | 1.1.0 | 1.1.0 | ACCEPT, 3 parsed / 1 skipped — one bad line must not zero the history |
| `events/v2_0_0_major_ahead.jsonl` | 2.0.0 (all records) | 1.1.0 | REFUSE, 0 parsed / 2 refused — whole-log refusal only when nothing survives |
| `events/v1_1_0_mixed_major_ahead.jsonl` | 1.1.0 + 2.0.0 | 1.1.0 | **ACCEPT_PARTIAL**, 2 parsed / 1 refused — the round-2 counterexample |
| `events/v1_1_0_mixed_minor_ahead.jsonl` | 1.1.0 + 1.2.0 | 1.1.0 | ACCEPT_FORWARD, 3 parsed / 0 refused |
| `events/v1_1_0_mixed_all_three.jsonl` | 1.1.0 + 1.2.0 + 2.0.0 + corrupt | 1.1.0 | **ACCEPT_PARTIAL**, 3 parsed / 1 skipped / 1 refused — baseline record *after* the major-ahead one still parses |



---

## 4. G-04 — the health-tool surface (no collision; no action)

**Finding: there is no `forge_get_health` name collision. No rename, no alias, no action in any repo.**

An earlier revision of this section asserted that both MCP servers register `forge_get_health` and
recommended renaming the plugin's tool to `forge_get_health_score` with a one-minor alias window. **That
premise was false and the recommendation is withdrawn.** It was inherited from the deep-dive G-04 line
and NEXUS:2082, both of which predate the plugin rename — it was never verified against the plugin repo.
Codex Wave-1 review caught it
(`ecosystem/forge/research/2026-07-18-codex-wave1-review.md`).

### 4.1 The actual cross-repo contract

Evidence tag: **forge-plugin `v3.10.2`** (`git show v3.10.2:plugins/nxtg-forge/servers/governance-mcp/index.mjs`).

| Surface | Tool name | Source | What it computes |
|---|---|---|---|
| forge-orchestrator (Rust MCP, 11 tools) | `forge_get_health` | `src/mcp/tools.rs:155` | 5-dimension **governance health** from `.forge/` state — docs, architecture, task health, knowledge, drift. Requires the `forge` binary and an initialized `.forge/` (L2). |
| forge-plugin governance-mcp (Node, 8 tools) | `forge_get_governance_health` | `index.mjs:56` (dispatch `:133`) | Repo-level **health score** from git/test/security probes. No `.forge/` dependency (L1). |

The names are already distinct at v3.10.2. The plugin's Node server registers **no** `forge_get_health`
and **no** alias for it, so when Claude Code loads both servers simultaneously, `forge_get_health`
resolves unambiguously to the orchestrator. There is no client-resolution-order hazard to mitigate.

Verified with: `git show v3.10.2:…/index.mjs | grep -oE 'forge_[a-z_]+' | sort -u` → the 8 plugin tools,
none named `forge_get_health`. The rename landed in plugin `v3.8.0` (`061716b`); the collision described
in the March-era notes was real then and is closed now.

The 9 plugin markdown files that mention `forge_get_health` are **correct, not stale** — each explicitly
documents the orchestrator's tool (e.g. `commands/status.md:61` "Orchestrator's 5-dimension health +
drift analysis (L2 only, requires the `forge` binary)"). They are describing the cross-server surface, not
calling a local tool. No cleanup is warranted, and flagging them would manufacture a replacement finding
where none exists.

### 4.2 What is worth keeping from the original position

The two health surfaces are **semantically distinct and neither substitutes for the other** — governance
health (orchestrator, `.forge/` state, L2) vs repo health score (plugin, git/test/security probes, L1).
That distinction is the durable content of this section, and it is why the current names are good ones:
they say which is which. Consumers picking a health number should choose on availability tier — L1
projects have no `.forge/`, so only the plugin's score exists there; L2 projects have both, and the
orchestrator's is canonical for anything drift- or task-related (this is the contract forge-ui is being
pointed at under G-03).

The orchestrator makes no change under G-04.

---

## 5. Open questions for the other two teams

1. **forge-ui**: does the dashboard tail `events.jsonl` directly, or only via `forge_get_events`? If
   directly, the skip-and-count rule (§3.2 rule 4) is a UI-side change too, not just an MCP one.
2. **forge-plugin**: is anything reading `ForgeState.version` today? If yes, the deprecation window in
   3.1 needs to be longer than one minor.
3. **Both**: is a MAJOR-mismatch refusal (3.2.2) acceptable UX in the dashboard, or should it degrade to
   a read-only banner? The orchestrator's position is refuse-and-name — but the surface that has to show
   it is forge-ui's, so forge-ui decides the presentation.
