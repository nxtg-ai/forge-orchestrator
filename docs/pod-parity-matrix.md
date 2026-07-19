# `forge pod` ↔ cosmux v0.4.2 Parity Matrix

**Status**: current as of forge-orchestrator `main`, DIRECTIVE-NXTG-20260718-09 checkpoint 3
**Reference**: cosmux v0.4.2 (`~/projects/cosmux`, Apache-2.0)
**Verified by**: `tests/pod_parity.rs` (14 live pod shapes) + `tests/doctor_integration.rs`

`forge pod` vendors cosmux's core and operates on the **same store and the same pod locations, in
place** — `~/.cosmux/state.json`, `~/.config/cosmux/templates/`, and the legacy search order. A
`forge pod` invocation and a `cosmux` invocation see the same files. There is no import step and no
new store path.

## Verb matrix — 11 public + 2 hidden

| Verb | Flags | Exit codes | Parity |
|---|---|---|---|
| `start <name>` | `--force`, `--attach` | 0 ok · 1 error | Full |
| `stop <name>` | — | 0 ok · 1 error | Full |
| `list` | — | 0 ok · 1 error | Full |
| `validate <name>` | — | 0 valid · 1 invalid | Full |
| `show <name>` | — | 0 ok · 1 error | Full |
| `state` (alias **`hud`**) | — | 0 ok · 1 error | Full — see alias note |
| `ps` | — | 0 ok · 1 error | Full |
| `gc` | — | 0 ok · 1 error | Full |
| `reload <name>` | `--force` | 0 ok · 1 error | Full |
| `completions <shell>` | bash·zsh·fish·powershell·elvish | 0 ok | Full |
| `preflight [pod]` | `--against <path>` | 0 covered · **2 uncovered or empty parse** · 1 error | Full |
| `_pane-recover <session>` | hidden | 0 ok · 1 error | Full |
| `_after-detach <session>` | hidden | 0 ok · 1 error | Full |

### `hud` is an alias, not a verb

cosmux registers `hud` as an alias of `state`, and nxtg users type both. It is preserved as an
alias here so **there is no behaviour change for them**. An earlier draft of the directive listed
`hud` as a 12th verb; it is not, and `tests/pod_parity.rs::hud_is_an_alias_of_state_not_a_separate_verb`
asserts the two produce identical output and exit codes.

### `preflight` exit 2 is load-bearing

`preflight` distinguishes **"ran and found gaps" (2)** from **"could not run" (1)**. An **empty
target set is a hard failure (2), never a pass** — a check that extracted nothing has not verified
coverage. That rule exists because of the 2026-04-19 NXTG-AI incident where a team went 9 hours
silent after a pod YAML omitted a window the heartbeat targeted.

## Deliberate deviations

| Area | cosmux | forge | Why |
|---|---|---|---|
| Binary / prefix | `cosmux <verb>` | `forge pod <verb>` | Namespaced under the existing CLI |
| Hook command | `cosmux _pane-recover` | `forge pod _pane-recover` | Hooks installed on pods **forge** spawns point at forge. Rebinding *existing* cosmux sessions is a separate, gated migration step. |
| Hook log path | `/tmp/cosmux-<session>.log` | `/tmp/forge-pod-<session>.log` | Avoids two tools appending to one file |
| Pane `task:` | — | optional `.forge` task binding | New capability; `skip_serializing_if` keeps it out of files cosmux reads |
| Date formatting | hand-rolled civil-date arithmetic | `chrono` | Same ISO-8601 `Z` output; forge already depends on chrono |
| Logging | `log` + `env_logger` | `tracing` | Already present in forge; messages preserved |
| `~user` expansion | `shellexpand::tilde` | leading `~` / `~/` only | Vendored to avoid a dependency; `~user` is unused by every pod config |

## Dependency delta

**+1 dependency total** (`clap_complete`), not +3:

| Dep | Disposition | Instrument |
|---|---|---|
| `clap_complete` | **added** | Required by the `completions` verb; FPL ruled the parity contract outranks dependency minimization |
| `shell-words` | **not added — dead upstream** | `grep -rn "shell_words\|shell-words" --include=*.rs` over the whole cosmux tree returns nothing. Declared in its `Cargo.toml`, referenced by zero lines of its source. |
| `shellexpand` | **not added — vendored** | One call site (`config.rs:110`). Replaced by `pod::config::expand_path`. |
| `log`, `env_logger` | **dropped** | → `tracing`, already a forge dependency |

## Test-isolation seams (production behaviour unchanged)

Both default to the live location, so production is byte-identical to cosmux. They exist because
the two live surfaces here are destructive to touch from a test.

| Env var | Default | Purpose |
|---|---|---|
| `FORGE_POD_STATE_DIR` | `~/.cosmux` | Redirect the store. Under `cfg(test)` a write outside the temp dir **panics** — an override alone is not enough when the default is destructive. |
| `FORGE_POD_TMUX_SOCKET` | *(unset — default server)* | `tmux -L <socket>`, so no test can reach the operator's real tmux server. |
| `FORGE_POD_TEMPLATE_DIR` | `~/.config/cosmux/templates` | Redirect template lookup. |

## How parity is verified

`tests/pod_parity.rs` runs against **copies** of the 14 live fleet pods
(`tests/fixtures/pods/`, copied read-only from `~/ASIF/infra/tmux/`):

- all 14 shapes validate;
- `show` output is byte-identical across repeated runs (a non-deterministic plan could not be
  compared against cosmux at all);
- every declared window and pane survives the resolve — a dropped window is precisely the incident
  `preflight` exists to catch;
- `validate` / `show` / `ps` / `state` leave the store byte-identical;
- `hud` and `state` are indistinguishable;
- `preflight` exits 2 on an empty target set;
- tmux absent produces a named error (`tmux not found`), exit 1, no panic.

Parity is asserted on the **side-effect-free** surface plus the pure `spawn_plan`, which computes
the full tmux argument sequence without executing anything. Asserting `start`/`stop` parity by
observing a real tmux server would require touching the live fleet sessions this work is
constrained not to touch.

## Not implemented (gated migration surface)

`forge pod adopt`, the `cosmux` shim, and hook-rebinding of **existing live sessions** are the
migration protocol, tracked separately. Installing hooks on a pod forge itself spawns is ordinary
`start` behaviour and is implemented; re-pointing sessions cosmux already created is not.
