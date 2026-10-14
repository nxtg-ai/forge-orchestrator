//! Reads the release stage from the root `STAGE` file (the one constant) at build time and
//! exposes the derived label as `FORGE_STAGE`, so `forge --version` and the MCP health tool cannot
//! drift from it.
//!
//! `STAGE` records the two registry axes, one per line: `distribution: internal|public` and
//! `maturity: pre-alpha|dogfood|alpha|beta|rc|ga`. The label is derived, never stored:
//! public + pre-alpha/dogfood -> "public preview"; public + alpha/beta/rc/ga -> that stage;
//! internal -> "internal".

const DISTRIBUTIONS: [&str; 2] = ["internal", "public"];
const MATURITIES: [&str; 6] = ["pre-alpha", "dogfood", "alpha", "beta", "rc", "ga"];

fn field<'a>(raw: &'a str, key: &str) -> &'a str {
    raw.lines()
        .find_map(|l| l.trim().strip_prefix(key)?.strip_prefix(':'))
        .map(str::trim)
        .unwrap_or_else(|| panic!("STAGE has no `{key}:` line"))
}

fn main() {
    println!("cargo:rerun-if-changed=STAGE");
    let raw = std::fs::read_to_string("STAGE").expect("root STAGE file is missing");
    let distribution = field(&raw, "distribution");
    let maturity = field(&raw, "maturity");
    assert!(
        DISTRIBUTIONS.contains(&distribution),
        "STAGE distribution must be one of {DISTRIBUTIONS:?}, found {distribution:?}"
    );
    assert!(
        MATURITIES.contains(&maturity),
        "STAGE maturity must be one of {MATURITIES:?}, found {maturity:?}"
    );
    let label = match (distribution, maturity) {
        ("internal", _) => "internal",
        (_, "pre-alpha" | "dogfood") => "public preview",
        (_, "alpha") => "alpha",
        (_, "beta") => "beta",
        (_, "rc") => "release candidate",
        _ => "ga",
    };
    println!("cargo:rustc-env=FORGE_STAGE={label}");
}
