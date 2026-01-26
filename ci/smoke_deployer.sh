#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

providers=(aws azure gcp k8s local)
strategy="iac-only"
tenant="acme"
environment="dev"

state_dir="$ROOT/.greentic/state"
deploy_dir="$state_dir/deploy"
runtime_dir="$state_dir/runtime"

echo "==> resetting runtime state"
rm -rf "$deploy_dir" "$runtime_dir"

echo "==> running smoke harness against placeholder packs"
for provider in "${providers[@]}"; do
  pack_path="providers/deployer/${provider}"

  echo "==> greentic-deployer plan --provider ${provider}"
  cargo run -p greentic-deployer --bin greentic-deployer -- plan \
    --provider "${provider}" \
    --strategy "${strategy}" \
    --tenant "${tenant}" \
    --environment "${environment}" \
    --pack "${pack_path}" \
    --providers-dir providers/deployer \
    --packs-dir dist

  echo "==> greentic-deployer apply --provider ${provider} (dry-run)"
  cargo run -p greentic-deployer --bin greentic-deployer -- apply \
    --provider "${provider}" \
    --strategy "${strategy}" \
    --tenant "${tenant}" \
    --environment "${environment}" \
    --pack "${pack_path}" \
    --providers-dir providers/deployer \
    --packs-dir dist \
    --dry-run \
    --yes

  verify_runner_diagnostics "${provider}"
done

expected_provider_artifact() {
  case "$1" in
    aws | azure | gcp) echo "main.tf" ;;
    k8s) echo "Chart.yaml" ;;
    local) echo "local.sh" ;;
    *) echo ""
  esac
}

verify_runner_diagnostics() {
  provider="$1"
  diag_dir="$deploy_dir/${provider}/${tenant}/${environment}"
  diag_json="$diag_dir/._deployer_invocation.json"
  runner_txt="$diag_dir/._runner_cmd.txt"

  if [[ ! -f "$diag_json" ]]; then
    echo "==> missing diagnostics for ${provider} at ${diag_json}"
    exit 1
  fi
  if [[ ! -f "$runner_txt" ]]; then
    echo "==> missing runner command log at ${runner_txt}"
    exit 1
  fi

  echo "==> diagnostics for ${provider}:"
  python3 - <<'PY' "$diag_json"
import json, sys

path = sys.argv[1]
with open(path) as fh:
    data = json.load(fh)
print(f"  pack_id={data['pack_id']} flow_id={data['flow_id']}")
cmd = data.get("runner_cmd", [])
if isinstance(cmd, list):
    print("  runner_cmd=" + " ".join(cmd))
else:
    print(f"  runner_cmd={cmd}")
PY
  echo "==> runner command log:"
  cat "$runner_txt"

  artifact_dir="$diag_dir"
  if [[ ! -f "$artifact_dir/README.md" ]]; then
    echo "==> missing README.md output for ${provider} at ${artifact_dir}/README.md"
    exit 1
  fi

  provider_artifact="$(expected_provider_artifact "$provider")"
  if [[ -n "$provider_artifact" && ! -f "$artifact_dir/$provider_artifact" ]]; then
    echo "==> missing ${provider_artifact} output for ${provider} at ${artifact_dir}/$provider_artifact"
    exit 1
  fi
  echo "==> placeholder outputs verified for ${provider}"
}
