# RFC-0001 — Schema-Version Handshake for `.forge/` Cross-Repo Artifacts

**Status**: PROPOSED (no implementation in v1.5.2)
**Author**: forge-orchestrator team
**Date**: 2026-07-18
**Origin**: DIRECTIVE-NXTG-20260718-03 items 3 & 4 — deep-dive gaps G-04, G-05
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

- `state.json` → `state_schema: "2.0.0"` at the document root.
- `events.jsonl` → `v: "1.0.0"` on **each record** (short key: this is a hot append path with one write
  per event; the field is repeated on every line).

Semantics:

| Change | Bump | Reader obligation |
|---|---|---|
| New optional field | MINOR | Ignore unknown fields (already true) |
| New enum variant (e.g. `EventType`) | MINOR | Must not error — see 3.2 |
| Field removed / retyped / semantics changed | MAJOR | Reader may refuse and must say so |
| Value-only change (new `dashboard_mode` string) | PATCH | None |

The existing `ForgeState.version` field is **deprecated, not repurposed** — it is ambiguous in the field
(D-1) and its current values (`1.0.0`/`1.1.0`) would collide with a fresh scheme. New readers use
`state_schema`; the old field is written unchanged for one minor cycle, then dropped in the next MAJOR.

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

### 3.3 Rollout

| Phase | Orchestrator | Consumers |
|---|---|---|
| P0 (next minor) | Add `#[serde(other)] Unknown`; write `state_schema` + per-record `v`; keep writing legacy `version` | No change required; existing readers keep working |
| P1 | — | forge-ui + governance-mcp implement the 4 rules; add a test that a synthetic future variant does not break the reader |
| P2 (next MAJOR) | Drop legacy `ForgeState.version`; reconcile `init.rs:230` | Readers already on `state_schema` |

Consumers are never blocked on the orchestrator, and the orchestrator is never blocked on consumers.

### 3.4 Test obligations

Non-negotiable at implementation time (test count never decreases):

- Round-trip an event carrying an unknown `event_type` → deserializes as `Unknown`, does not error.
- Read a version-less `state.json` and a version-less `events.jsonl` → both accepted as `1.0.0`.
- Read a `state.json` with a MAJOR ahead of the reader → refused with an error naming both versions.
- Parse an `events.jsonl` with one corrupt line among valid ones → valid records returned, skip counted.

---

## 4. Position on G-04 — `forge_get_health` name collision

**Recommendation: rename on the plugin side to `forge_get_health_score`. Do not alias.**

Both MCP servers register a tool named `forge_get_health` and both are connected simultaneously by
forge-plugin's `.mcp.json`. They return different shapes for the same name — the orchestrator's is the
5-dimension governance health computed from `.forge/` state; the plugin's Node server computes a
repo-level score from git/test/security probes. Which one an agent reaches depends on client-side
resolution order, which is not a contract either repo controls.

Why rename rather than alias: an alias keeps the ambiguous name resolvable, so the failure it is meant to
fix stays reachable — and the resolution order that decides the winner still is not ours. The collision
must stop existing, not become survivable.

Why the plugin side moves: the orchestrator's tool is the **canonical** health surface (it is the one
`forge_get_health`'s 5-dimension governance score is documented against, and the one forge-ui's health
contract is being pointed at per G-03), and the plugin's is a repo-metrics score whose name is more
accurately `forge_get_health_score` anyway. NEXUS:2082 already proposed exactly this, scoped at 6 files.
Renaming the orchestrator instead would break the wider blast radius.

Sequencing, since MCP tool names are an agent-visible contract:

1. forge-plugin ships `forge_get_health_score` **and** keeps `forge_get_health` for one minor, with the
   old name's description prefixed `DEPRECATED — use forge_get_health_score`.
2. Plugin commands/agents that call it are updated in the same release (the 6 files).
3. Next plugin minor removes `forge_get_health` from the Node server. The name then resolves
   unambiguously to the orchestrator.

The orchestrator makes **no change** — this is a plugin-side rename. Recorded here because the directive
asked this repo for a position, and because the two servers' health semantics should be documented as
distinct surfaces regardless of naming: governance health (orchestrator, `.forge/` state) vs repo health
score (plugin, git/test/security probes). Neither is a substitute for the other.

---

## 5. Open questions for the other two teams

1. **forge-ui**: does the dashboard tail `events.jsonl` directly, or only via `forge_get_events`? If
   directly, the skip-and-count rule (3.2.4) is a UI-side change too, not just an MCP one.
2. **forge-plugin**: is anything reading `ForgeState.version` today? If yes, the deprecation window in
   3.1 needs to be longer than one minor.
3. **Both**: is a MAJOR-mismatch refusal (3.2.2) acceptable UX in the dashboard, or should it degrade to
   a read-only banner? The orchestrator's position is refuse-and-name — but the surface that has to show
   it is forge-ui's, so forge-ui decides the presentation.
