#!/usr/bin/env bash
# Runs the full quality gate: formatting, lints and tests.
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

echo "==> cargo fmt --check"
cargo fmt --all --check

# --all-targets is essential: without it clippy skips integration tests,
# benches and examples, and a broken test file passes the gate unnoticed.
echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test --workspace"
cargo test --workspace

echo "==> all checks passed"
