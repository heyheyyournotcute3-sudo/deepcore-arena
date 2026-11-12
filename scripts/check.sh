#!/usr/bin/env bash
# Runs the full quality gate: formatting, lints and tests.
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

echo "==> cargo fmt --check"
cargo fmt --all --check

echo "==> cargo clippy --workspace -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test --workspace"
cargo test --workspace

echo "==> all checks passed"
