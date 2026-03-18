# Contributing to forge-orchestrator

We welcome contributions! Before submitting a pull request, please review the following.

## Contributor License Agreement (CLA)

All contributors must sign our [Contributor License Agreement](.github/CLA.md) before their pull request can be merged. The CLA bot will automatically comment on your PR with instructions. Simply reply with the required comment to sign.

The CLA grants Apache ICLA-equivalent terms, ensuring contributions can be distributed under the project's license.

## Development Setup

```bash
# Build
cargo build

# Test (all tests must pass)
cargo test

# Lint (must pass with zero warnings)
cargo fmt
cargo clippy -- -D warnings
```

## Pull Request Requirements

1. **CLA signed** - the CLA bot must show a green check
2. **Tests pass** - `cargo test` must pass with all tests green
3. **Lint clean** - `cargo fmt --check` and `cargo clippy -- -D warnings` must pass
4. **Conventional commits** - use `feat:`, `fix:`, `docs:`, `chore:` prefixes

## Code Style

- Run `cargo fmt` before committing
- No clippy warnings (CI enforces `-D warnings`)
- Stage files explicitly (no `git add -A`)
- Write tests for new functionality

## License

By contributing, you agree that your contributions will be licensed under the project's [FSL-1.1-ALv2 license](LICENSE.md), which converts to Apache License 2.0 after two years.
