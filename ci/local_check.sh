#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> cargo fmt --check"
cargo fmt --all -- --check

echo "==> cargo clippy"
cargo clippy --all-targets --all-features -- -D warnings

echo "==> cargo test"
cargo test --all

echo "==> cargo doc"
cargo doc --no-deps

echo "==> scripts/ci-smoke.sh"
./scripts/ci-smoke.sh

echo "Local check completed successfully."
