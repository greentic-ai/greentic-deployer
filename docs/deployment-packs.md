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
