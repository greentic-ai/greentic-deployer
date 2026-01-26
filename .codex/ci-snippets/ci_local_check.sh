#!/usr/bin/env bash
set -euo pipefail

./scripts/gen_placeholders.sh

for p in dist/*.gtpack; do
  greentic-pack doctor --pack "$p"
done

# run deployer smoke (adjust to your deployer CLI)
# ./ci/smoke_deployer.sh
