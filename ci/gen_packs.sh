#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PACKGEN_CMD=(
  cargo
  run
  -p
  greentic-deployer
  --bin
  greentic-deployer-packgen
  --
)

providers=(aws azure gcp k8s local generic)

rm -rf "$ROOT/providers/deployer" "$ROOT/dist"
mkdir -p "$ROOT/providers/deployer" "$ROOT/dist"

echo "==> generating deployer provider packs via greentic-deployer-packgen"
for provider in "${providers[@]}"; do
  echo "==> generating provider ${provider}"
  "${PACKGEN_CMD[@]}" generate \
    --provider "${provider}" \
    --out providers/deployer \
    --dist dist
done

echo "==> packgen completed"
