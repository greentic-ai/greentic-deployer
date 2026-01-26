#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACK_MANIFEST="$ROOT/../greentic-pack/Cargo.toml"
FLOW_MANIFEST="$ROOT/../greentic-flow/Cargo.toml"
PACK_TARGET_DIR="$ROOT/.target/greentic-pack"
FLOW_TARGET_DIR="$ROOT/.target/greentic-flow"

PACK_CMD=(
  cargo
  run
  --manifest-path
  "$PACK_MANIFEST"
  --bin
  greentic-pack
  --
)
FLOW_CMD=(
  cargo
  run
  --manifest-path
  "$FLOW_MANIFEST"
  --bin
  greentic-flow
  --
)

run_pack() {
  CARGO_TARGET_DIR="$PACK_TARGET_DIR" "${PACK_CMD[@]}" "$@"
}

run_flow() {
  CARGO_TARGET_DIR="$FLOW_TARGET_DIR" "${FLOW_CMD[@]}" "$@"
}

providers=(aws azure gcp k8s local generic)
pack_prefix="greentic.demo.deploy"
pack_dir="$ROOT/providers/deployer"
dist_dir="$ROOT/dist"

mkdir -p "$PACK_TARGET_DIR" "$FLOW_TARGET_DIR"
rm -rf "$pack_dir"
mkdir -p "$pack_dir"
rm -rf "$dist_dir"
mkdir -p "$dist_dir"

echo "==> building placeholder packs via greentic-pack + greentic-flow"
for provider in "${providers[@]}"; do
  pack_id="${pack_prefix}.${provider}"
  target="${pack_dir}/${provider}"
  echo "==> scaffolding pack ${pack_id}"
  rm -rf "$target"
  run_pack new "$pack_id" --dir "$target"
  rm -f "$target/flows/main.ygtc" "$target/flows/main.ygtc.resolve.json"

  flow_file="$target/flows/deploy_${provider}_iac.ygtc"
  echo "==> creating flow deploy_${provider}_iac"
  run_flow new \
    --flow "$flow_file" \
    --id "deploy_${provider}_iac" \
    --type component-config \
    --schema-version 2

  run_flow doctor "$target/flows"
  run_pack update --in "$target"

  gtpack="$dist_dir/${pack_id}.gtpack"
  run_pack build --in "$target" --gtpack-out "$gtpack"
  run_pack doctor --pack "$gtpack"

  cat <<EOF >"$target/README.md"
# ${pack_id}

Provider: ${provider}

This pack emits placeholder IaC via \`deploy_${provider}_iac\`.
EOF

  echo "generated ${pack_id} (provider ${provider})"
done
