#!/usr/bin/env bash
set -euo pipefail

if command -v cargo-nextest >/dev/null 2>&1; then
    cargo nextest run --workspace --all-features
else
    echo "cargo-nextest is unavailable; falling back to cargo test"
    cargo test --workspace --all-features
fi

cargo test --doc --workspace --all-features
