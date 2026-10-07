//! Reads the release stage from the root `STAGE` file (the one constant) at build time and
//! exposes it as `FORGE_STAGE`, so `forge --version` and the MCP health tool cannot drift from it.

const STAGES: [&str; 6] = ["internal", "dogfood", "alpha", "beta", "rc", "ga"];

fn main() {
    println!("cargo:rerun-if-changed=STAGE");
    let raw = std::fs::read_to_string("STAGE").expect("root STAGE file is missing");
    let stage = raw.trim();
    assert!(
        STAGES.contains(&stage),
        "STAGE must be one of {STAGES:?}, found {stage:?}"
    );
    println!("cargo:rustc-env=FORGE_STAGE={stage}");
}
