#!/usr/bin/env python3
"""Conformance checker for RFC-0001 schema-version fixtures.

This is the *executable form of the comparison rule* in RFC-0001 §3.2.1 — deliberately
NOT an implementation of the handshake in forge-orchestrator/src/. Nothing in the
orchestrator reads or writes a schema version until the RFC is ratified. The point of
this script is that all three repos can verify they agree on the same matrix before
anyone writes reader code.

Usage:
    python3 check_fixtures.py            # run every case in expectations.json
    python3 check_fixtures.py -v         # also print the reasoning for each case

Exit code 0 = every case matches. Non-zero = at least one mismatch (printed).
No third-party dependencies; stdlib only.
"""

import argparse
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).parent

ACCEPT = "ACCEPT"
ACCEPT_FORWARD = "ACCEPT_FORWARD"
ACCEPT_PARTIAL = "ACCEPT_PARTIAL"
REFUSE = "REFUSE"


def parse_version(raw):
    """'1.2.3' -> (1, 2, 3). Raises ValueError on anything that is not 3 ints."""
    parts = raw.split(".")
    if len(parts) != 3:
        raise ValueError(f"not a 3-part semver: {raw!r}")
    return tuple(int(p) for p in parts)


def verdict(supported, file_version):
    """RFC-0001 §3.2.1, normative.

    Only MAJOR and MINOR decide compatibility; PATCH never does.
    Returns (verdict, message). The message names both versions on REFUSE so a
    refusal is never a silent empty read.
    """
    s = parse_version(supported)
    f = parse_version(file_version)

    if f[0] > s[0]:
        return REFUSE, (
            f"file is schema {file_version}, this reader supports {supported} "
            "(file newer by a major version — shape may have changed incompatibly)"
        )
    if f[0] < s[0]:
        return REFUSE, (
            f"file is schema {file_version}, this reader supports {supported} "
            "(file older major — reader cannot assume the old shape is a subset)"
        )
    if f[1] > s[1]:
        return ACCEPT_FORWARD, (
            f"file is schema {file_version}, this reader supports {supported} "
            "(minor ahead — unknown fields and variants tolerated)"
        )
    return ACCEPT, f"file is schema {file_version}, reader supports {supported}"


def read_state(path, baseline):
    """Returns (file_version, unknown_root_fields_relative_to_baseline_shape)."""
    doc = json.loads(path.read_text())
    file_version = doc.get("state_schema", baseline)
    known = {
        "state_schema", "version", "project_name", "created_at", "updated_at",
        "tools", "brain", "task_summary", "active_locks", "agent_auth",
        "agent_permissions", "git", "scheduler", "dashboard_mode", "current_phase",
    }
    unknown = sorted(k for k in doc if k not in known)
    return file_version, unknown


def read_events(path, supported, baseline, known_event_types):
    """Per-record version comparison + batch semantics, RFC-0001 §3.2.2.

    An events.jsonl is append-only and therefore inherently mixed-version (§1 D-2):
    one file can hold records written by several forge releases. EVERY record's own
    `v` is compared; a single verdict for the whole log would silently trust a
    major-ahead record appended after compatible history.

    Returns a dict with the batch verdict, per-record counts, and the versions seen.
    """
    parsed = 0          # records accepted into the result set
    skipped = 0         # malformed lines (§3.2 rule 4)
    refused = 0         # records quarantined as major-incompatible (§3.2.2)
    unknown_variants = []
    versions_seen = []
    refused_versions = []
    forward = False

    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            # One bad line must not zero out the history.
            skipped += 1
            continue

        rec_version = rec.get("v", baseline)
        if rec_version not in versions_seen:
            versions_seen.append(rec_version)

        rec_verdict, _ = verdict(supported, rec_version)
        if rec_verdict == REFUSE:
            # Quarantined, not parsed: this reader cannot trust the shape.
            refused += 1
            if rec_version not in refused_versions:
                refused_versions.append(rec_version)
            continue
        if rec_verdict == ACCEPT_FORWARD:
            forward = True

        parsed += 1
        et = rec.get("event_type")
        if et is not None and et not in known_event_types and et not in unknown_variants:
            unknown_variants.append(et)

    # Batch verdict (§3.2.2). Refusing the whole log because one future record
    # appeared would zero a history — the failure §3.2 rule 4 exists to prevent.
    if refused and parsed == 0:
        batch = REFUSE          # nothing in the file is readable by this reader
    elif refused:
        batch = ACCEPT_PARTIAL  # compatible history readable, incompatible tail counted
    elif forward:
        batch = ACCEPT_FORWARD
    else:
        batch = ACCEPT

    return {
        "verdict": batch,
        "parsed": parsed,
        "skipped": skipped,
        "refused": refused,
        "unknown_variants": sorted(unknown_variants),
        "versions_seen": sorted(versions_seen),
        "refused_versions": sorted(refused_versions),
    }


def check_case(case, spec, verbose):
    """Returns a list of failure strings (empty = pass)."""
    baseline = spec["absent_version_baseline"]
    path = HERE / case["fixture"]
    failures = []

    if not path.exists():
        return [f"fixture missing: {case['fixture']}"]

    if case["artifact"] == "state":
        # state.json is rewritten whole, so it has exactly one version.
        file_version, unknown_fields = read_state(path, baseline)
        if file_version != case["expected_file_version"]:
            failures.append(
                f"file version: expected {case['expected_file_version']}, got {file_version}"
            )
        got_verdict, message = verdict(case["supported_version"], file_version)

        for needle in case.get("expected_message_contains", []):
            if needle not in message:
                failures.append(f"refusal message must name {needle!r}; got: {message}")

        expected_unknown_fields = case.get("expected_unknown_fields")
        if expected_unknown_fields is not None and unknown_fields != expected_unknown_fields:
            failures.append(
                f"unknown fields: expected {expected_unknown_fields}, got {unknown_fields}"
            )
    else:
        # events.jsonl is append-only and mixed-version: every record is compared.
        result = read_events(
            path, case["supported_version"], baseline, set(spec["known_event_types"])
        )
        got_verdict = result["verdict"]

        expected_versions = case.get("expected_versions_seen")
        if expected_versions is not None and result["versions_seen"] != sorted(expected_versions):
            failures.append(
                f"versions seen: expected {sorted(expected_versions)}, got {result['versions_seen']}"
            )

        for key in ("parsed", "skipped", "refused"):
            expected_key = f"expected_{key}"
            if expected_key in case and result[key] != case[expected_key]:
                failures.append(
                    f"{key}: expected {case[expected_key]}, got {result[key]}"
                )

        expected_refused_versions = case.get("expected_refused_versions")
        if (
            expected_refused_versions is not None
            and result["refused_versions"] != sorted(expected_refused_versions)
        ):
            failures.append(
                f"refused versions: expected {sorted(expected_refused_versions)}, "
                f"got {result['refused_versions']}"
            )

        expected_unknown_variants = case.get("expected_unknown_variants")
        if (
            expected_unknown_variants is not None
            and result["unknown_variants"] != expected_unknown_variants
        ):
            failures.append(
                f"unknown variants: expected {expected_unknown_variants}, "
                f"got {result['unknown_variants']}"
            )

    if got_verdict != case["expected_verdict"]:
        failures.append(f"verdict: expected {case['expected_verdict']}, got {got_verdict}")

    if verbose and not failures:
        print(f"      {case['why']}")

    return failures


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("-v", "--verbose", action="store_true", help="print case rationale")
    args = ap.parse_args()

    spec = json.loads((HERE / "expectations.json").read_text())

    # The baseline rule is itself part of the spec: absent => 1.0.0, first
    # versioned release => 1.1.0 (adding the field is a MINOR change).
    baseline = parse_version(spec["absent_version_baseline"])
    first = parse_version(spec["first_versioned_release"])
    if not (first[0] == baseline[0] and first[1] == baseline[1] + 1):
        print(
            f"SPEC INCONSISTENT: first_versioned_release {spec['first_versioned_release']} "
            f"is not one MINOR above absent_version_baseline {spec['absent_version_baseline']} — "
            "adding the version field is a MINOR change per RFC-0001 §3.1",
            file=sys.stderr,
        )
        return 2

    total = len(spec["cases"])
    failed = 0
    for case in spec["cases"]:
        failures = check_case(case, spec, args.verbose)
        if failures:
            failed += 1
            print(f"FAIL  {case['id']}")
            for f in failures:
                print(f"      - {f}")
        else:
            print(f"ok    {case['id']}")

    print()
    if failed:
        print(f"{failed} of {total} cases FAILED")
        return 1
    print(f"all {total} cases passed — fixtures conform to RFC-0001 §3.2.1")
    return 0


if __name__ == "__main__":
    sys.exit(main())
