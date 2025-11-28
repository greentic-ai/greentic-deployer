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
