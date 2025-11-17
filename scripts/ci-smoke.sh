#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

providers=(aws azure gcp)
actions=(apply destroy)
packs=("acme:examples/acme-pack" "acmeplus:examples/acme-plus-pack")

for provider in "${providers[@]}"; do
  for pack_entry in "${packs[@]}"; do
    tenant="${pack_entry%%:*}"
    pack_path="${pack_entry#*:}"
    for action in "${actions[@]}"; do
      echo "==> greentic-deployer ${action} --provider ${provider} --tenant ${tenant} (dry-run)"
      cargo run -p greentic-deployer -- "${action}" \
        --provider "${provider}" \
        --tenant "${tenant}" \
        --environment staging \
        --pack "${pack_path}" \
        --dry-run \
        --yes
    done
  done
done
