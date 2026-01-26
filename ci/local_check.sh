#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

state_dir="$ROOT/.greentic/state"
rm -rf "$state_dir/deploy" "$state_dir/runtime"

if ! command -v cargo-binstall &>/dev/null; then
  echo "cargo-binstall not found; installing via cargo"
  cargo install --locked cargo-binstall
fi
echo "==> installing greentic CLIs via cargo-binstall"
cargo binstall --force greentic-component greentic-flow greentic-pack

echo "==> ci/gen_packs.sh"
./ci/gen_packs.sh

echo "==> cargo fmt --check"
cargo fmt --all -- --check

echo "==> cargo clippy"
cargo clippy --all-targets --all-features -- -D warnings

echo "==> cargo test"
cargo test --all

echo "==> cargo doc"
cargo doc --no-deps

echo "==> greentic-pack doctor dist/*.gtpack"
for pack in dist/*.gtpack; do
  greentic-pack doctor --pack "$pack"
done

echo "==> ci/smoke_deployer.sh"
./ci/smoke_deployer.sh

echo "Local check completed successfully."
