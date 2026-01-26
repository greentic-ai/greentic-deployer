# Deployment Packs

Deployment packs extend Greentic deployments without changing `greentic-deployer`’s code. They are regular Greentic packs with:

- `kind: "deployment"`.
- One or more `type: events` flows acting as deployment pipelines.
- Nodes built from deployment components (`supports: ["events"]`, `world: "greentic:deploy-plan@1.0.0"`).
- Host IaC permissions (`host.iac.write_templates = true` and optionally `host.iac.execute_plans`).

## Provider / strategy mapping

`greentic-deployer` maps `(--provider, --strategy)` to `(deployment_pack_id, deploy_flow_id)`. Examples:

| Provider | Strategy   | Deployment pack          | Flow ID               |
|----------|------------|--------------------------|-----------------------|
| `aws`    | `serverless` | `greentic.deploy.aws`     | `deploy_aws_serverless` |
| `generic`| `iac`      | `greentic.deploy.generic` | `deploy_generic_iac`  |

The mapping can be extended without recompiling the deployer—new targets just publish new deployment packs.

## Runtime integration

1. Load the **application pack** (kind `application`/`mixed`).
2. Build a provider-agnostic `DeploymentPlan`.
3. Resolve the deployment pack for `(provider,strategy)` and load it via `greentic-pack`.
4. Execute the deployment flow with `greentic-runner`, providing:
   - `ExecutionCtx.deployment_plan = Some(plan)` so components call `get-deployment-plan()`.
   - A filesystem preopen (e.g. host `deploy/<provider>/<tenant>/<env>/` mounted at guest `/iac`).

Deployment packs then generate IaC/templates, secrets manifests, and any other provider-specific assets entirely in Wasm, keeping `greentic-deployer` and the shared types provider-agnostic.

## Roles, profiles, and targets

`greentic-deployer` now carries a lightweight planning model per component:

- **Role:** `event_provider`, `event_bridge`, `messaging_adapter`, `worker`, or `other`.
- **Deployment profile:** `long_lived_service`, `http_endpoint`, `queue_consumer`, `scheduled_source`, `one_shot_job` (extensible).
- **Target:** `local`, `aws`, `azure`, `gcp`, `k8s`.

Profiles come from pack metadata when present (e.g. `greentic.deployment.profiles[component_id]`). When missing, the deployer infers profiles from worlds + tags (`http-endpoint`, `scheduled`, `queue-consumer`, `long-lived`, `one-shot`) and falls back conservatively with a warning (defaulting to `long_lived_service`).

Each profile maps to target-specific infra primitives, still without knowing any provider brands:

- `http_endpoint`: local gateway/handler, API Gateway+Lambda, Function App (HTTP), Cloud Run (HTTP), Ingress+Service+Deployment.
- `long_lived_service`: runner-managed process, ECS/EKS service, Container Apps/App Service, Cloud Run (always-on), Deployment+Service.
- `queue_consumer`: local queue worker, SQS+Lambda, Service Bus trigger, Pub/Sub subscriber, Deployment-based consumer.
- `scheduled_source`: local scheduler, EventBridge+Lambda, Timer Function, Cloud Scheduler+Run/Function, CronJob.
- `one_shot_job`: runner one-shot, Lambda, Container Apps job/Function, Cloud Run job, Job.

Local/K8s note: the legacy Rust backends only cover AWS/Azure/GCP. For `local` and `k8s` targets you need a deployment pack plus a registered executor (or extend the provider/strategy mapping to point at your pack) so the deployer can delegate IaC generation.

CLI plans now surface these mappings (and inference warnings) and can be rendered as text, JSON, or YAML via `--output text|json|yaml`.

## Authoring flows

For the YGTC flow authoring format and schema notes, see `docs/flow-authoring.md`.

## Loading packs

- Local files/directories: `--pack <path>` continues to work.
- Registry/distributor: use `--pack-id`, `--pack-version`, and `--pack-digest` plus `--distributor-url` (and optionally `--distributor-token`) to resolve packs from a distributor. Programmatic callers can also register a distributor source via `set_distributor_source`.
- The default HTTP source posts to `/distributor-api/pack` with `pack_id` and `version` and retries on transient errors.

### Notes for distributors

- The HTTP fetcher is intentionally minimal. If your distributor API differs, register a custom `DistributorSource` via `set_distributor_source` before calling `build_plan`.
- `reqwest` is used in blocking mode by default; swap in an async source if needed.

# Placeholder pack pipeline

`greentic-deployer` now exposes a deterministic pipeline backed by `greentic-deployer-packgen`:

1. `ci/gen_packs.sh` runs `greentic-deployer-packgen generate --provider <name>` for every placeholder provider (`aws`, `azure`, `gcp`, `k8s`, `local`, `generic`). Packgen orchestrates the canonical CLIs (`greentic-pack new/add-extension/build/doctor`, `greentic-flow new/doctor`, optionally `greentic-component` tooling) so every pack is scaffolded, validated, and emitted as `dist/greentic.demo.deploy.<provider>.gtpack`.
2. `ci/smoke_deployer.sh` exercises every placeholder pack with `plan` + `apply --dry-run`, then inspects `.greentic/state/deploy/<provider>/<tenant>/<environment>` for `._deployer_invocation.json` and `._runner_cmd.txt` to surface the resolved `(pack_id, flow_id)` and runner command.
3. `ci/local_check.sh` bundles fmt/clippy/tests/doc checks, installs the greentic CLIs via `cargo binstall`, runs `ci/gen_packs.sh`, and re-runs `greentic-pack doctor` plus the smoke harness so the ABI and diagnostics stay locked to real packs.
4. `.github/workflows/ci.yml` drives the same steps through the `local-check`, `build`, `doctor`, and `smoke` jobs so the `publish` job for tags only runs after every pack has been generated, doctored, and smoke-tested.

Keeping this pipeline green ensures every placeholder pack shipped via GHCR is CLI-built, doctored, and validated by the smoke harness with deterministic logs.
