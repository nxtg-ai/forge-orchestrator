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


def read_events(path, baseline, known_event_types):
    """Skip-and-count per RFC-0001 §3.2 rule 4.

    Returns (file_version, parsed, skipped, unknown_variants). The file version is
    taken from the first parseable record; a log with no v is baseline.
    """
    file_version = None
    parsed = 0
    skipped = 0
    unknown_variants = []

    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            # One bad line must not zero out the history.
            skipped += 1
            continue
        parsed += 1
        if file_version is None:
            file_version = rec.get("v", baseline)
        et = rec.get("event_type")
        if et is not None and et not in known_event_types and et not in unknown_variants:
            unknown_variants.append(et)

    return (file_version or baseline), parsed, skipped, sorted(unknown_variants)


def check_case(case, spec, verbose):
    """Returns a list of failure strings (empty = pass)."""
    baseline = spec["absent_version_baseline"]
    path = HERE / case["fixture"]
    failures = []

    if not path.exists():
        return [f"fixture missing: {case['fixture']}"]

    if case["artifact"] == "state":
        file_version, unknown_fields = read_state(path, baseline)
        parsed = skipped = None
        unknown_variants = []
    else:
        file_version, parsed, skipped, unknown_variants = read_events(
            path, baseline, set(spec["known_event_types"])
        )
        unknown_fields = []

    if file_version != case["expected_file_version"]:
        failures.append(
            f"file version: expected {case['expected_file_version']}, got {file_version}"
        )

    got_verdict, message = verdict(case["supported_version"], file_version)
    if got_verdict != case["expected_verdict"]:
        failures.append(
            f"verdict: expected {case['expected_verdict']}, got {got_verdict} ({message})"
        )

    for needle in case.get("expected_message_contains", []):
        if needle not in message:
            failures.append(f"refusal message must name {needle!r}; got: {message}")

    # A REFUSE verdict means the reader stops before trusting record contents,
    # so record-level expectations only apply to the accepting verdicts.
    if got_verdict != REFUSE:
        if "expected_parsed" in case and parsed != case["expected_parsed"]:
            failures.append(f"parsed: expected {case['expected_parsed']}, got {parsed}")
        if "expected_skipped" in case and skipped != case["expected_skipped"]:
            failures.append(f"skipped: expected {case['expected_skipped']}, got {skipped}")

    expected_unknown_fields = case.get("expected_unknown_fields")
    if expected_unknown_fields is not None and unknown_fields != expected_unknown_fields:
        failures.append(
            f"unknown fields: expected {expected_unknown_fields}, got {unknown_fields}"
        )

    expected_unknown_variants = case.get("expected_unknown_variants")
    if expected_unknown_variants is not None and unknown_variants != expected_unknown_variants:
        failures.append(
            f"unknown variants: expected {expected_unknown_variants}, got {unknown_variants}"
        )

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
