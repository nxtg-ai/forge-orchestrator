# Contributing to forge-orchestrator

We welcome contributions! Before submitting a pull request, please review the following.

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

1. **Tests pass** - `cargo test` must pass with all tests green
2. **Lint clean** - `cargo fmt --check` and `cargo clippy -- -D warnings` must pass
3. **Conventional commits** - use `feat:`, `fix:`, `docs:`, `chore:` prefixes

## Code Style

- Run `cargo fmt` before committing
- No clippy warnings (CI enforces `-D warnings`)
- Stage files explicitly (no `git add -A`)
- Write tests for new functionality

## License

By contributing, you agree that your contributions will be licensed under the project's [Apache License, Version 2.0](LICENSE.md).
