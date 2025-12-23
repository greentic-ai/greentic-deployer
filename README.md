# Greentic Deployer

`greentic-deployer` is a CLI and library that builds cloud-agnostic deployment plans for Greentic packs and then runs provider-specific *deployment packs* (kind `deployment`) to materialise the required IaC/artifacts for AWS, Azure, GCP, local, K8s, or any other target.

## Concepts

- **Application packs** (kind `application` or `mixed`) describe flows, components, tools, secrets, and tenant bindings. `greentic-deployer` introspects them to understand runners, messaging, channels, secrets, OAuth, and telemetry requirements.
- **DeploymentPlan** is a provider-agnostic model (`greentic-types::DeploymentPlan`) that captures messaging topology, runner services, channel ingress, secrets, OAuth redirect URLs, and telemetry hooks.
- **Deployment packs** (kind `deployment`) supply provider-specific deployment flows. Each flow is a `type: events` flow made of deployment components (`supports: ["events"]`, `world: "greentic:deploy-plan@1.0.0"`) that can read the plan via `get-deployment-plan()` and emit IaC/templates when the host grants `host.iac.write_templates`.
- **Providers / strategies** are a mapping (`provider`, `strategy`) → `(deployment_pack_id, deploy_flow_id)`; e.g. `("aws","serverless") -> ("greentic.deploy.aws","deploy_aws_serverless")`. `greentic-deployer` chooses the mapping for the requested `--provider`/`--strategy`, loads that deployment pack, and executes it via `greentic-runner`.
- Legacy Rust backends exist for AWS/Azure/GCP; Local/K8s currently require deployment packs + a registered executor and will return `DeploymentPackUnsupported` if no executor is present.

## Building

```bash
cargo build -p greentic-deployer
```

## CLI

```
greentic-deployer <plan|apply|destroy> --provider <local|aws|azure|gcp|k8s> \
  --tenant <tenant-id> --environment <env> --pack <path> \
  [--yes] [--preview] [--dry-run] [--iac-tool <tf|terraform|tofu|opentofu>] \
  [--output <text|json|yaml>]
```

Examples:

- Plan an AWS deployment:
  ```bash
greentic-deployer plan --provider aws --tenant acme --environment staging --pack examples/acme-pack
```
- Apply the plan once reviewed:
  ```bash
greentic-deployer apply --provider aws --tenant acme --environment staging --pack examples/acme-pack --yes
```
- Destroy resources when you no longer need them:
  ```bash
  greentic-deployer destroy --provider aws --tenant acme --environment staging --pack examples/acme-pack
  ```
- Plan locally (requires a deployment pack + executor for local targets):
  ```bash
  greentic-deployer plan --provider local --tenant acme --environment dev --pack examples/acme-pack --output json
  ```
- Resolve packs from a distributor/registry instead of the filesystem:
  ```bash
  greentic-deployer plan \
    --provider aws \
    --tenant acme \
    --environment staging \
    --pack-id dev.greentic.sample \
    --pack-version 0.1.0 \
    --pack-digest sha256:deadbeef \
    --distributor-url https://distributor.example.com \
    --distributor-token $DISTRIBUTOR_TOKEN
  ```

Plans and provider artifacts are written to `deploy/<provider>/<tenant>/<environment>/` for inspection.
Plan output also lists component role/profile mappings per target; use `--output json` or `--output yaml` for machine-readable summaries.
For Local/K8s targets, wire in a deployment pack + executor (or extend the provider mapping) because legacy shims are only available for AWS/Azure/GCP.

## Configuration

- Configuration is resolved via `greentic-config` with precedence `CLI > env > project (.greentic/config.toml) > user (~/.config/greentic/config.toml) > defaults`. If you pass `--config <path>`, that file replaces project discovery; precedence becomes `CLI > env > explicit file > user > defaults`. Use `--config <path>` for an explicit file and `--explain-config`/`--explain-config-json` to print the resolved config/provenance.
- `deployer.base_domain` in config controls the domain used when emitting OAuth redirect URLs and channel ingress; defaults to `deploy.greentic.ai` via greentic-config.
- OTLP tracing reads the endpoint from config; `OTEL_EXPORTER_OTLP_ENDPOINT` remains a fallback.
- IaC tool selection comes from `--iac-tool` (Terraform/OpenTofu) or PATH auto-detection (prefers tofu).
- When `connection` is set to `Offline` in config, remote pack/distributor access is blocked unless `--allow-remote-in-offline` is provided.

## Secrets & OAuth

- Secret requirements are pulled from pack metadata (`secret_requirements`) and surfaced in plans. Apply/destroy preflight each required secret via the secrets-store and fail fast with the missing key list plus a remediation hint (`greentic-secrets init --pack <pack>`). No secret values are logged.
- `greentic-deployer` resolves secrets using the runtime tenant/environment scope; apply/destroy fail if the secrets-store does not contain the required entries.
- OAuth clients use `greentic-oauth`’s `ProviderId` identifiers (e.g. `google`, `microsoft`, `github`) so downstream tooling can reuse the same descriptors when wiring the broker, and redirect URLs continue to follow the `https://{domain}/oauth/{provider}/callback/{tenant}/{environment}` pattern.

-## Telemetry & Provider Artifacts

- Telemetry is instrumented via `greentic-telemetry`, which publishes OTLP spans for each `plan`, `apply`, or `destroy` action and injects a task-local `TelemetryCtx` capturing tenant/provider/session keys.
- Provider artifacts now embed the telemetry endpoint and context in the generated shell/HashiCorp/Deployment Manager snippets (for example, Terraform output includes `OTEL_EXPORTER_OTLP_ENDPOINT`, Azure Bicep adds the value under container `env`, and GCP config adds the annotation metadata), so every generated service inherits the tenant context.
- Secrets, OAuth redirects, and binding hints are surfaced directly inside the provider outputs so you can see which vault entries and redirect URLs will be consumed up front.
- OAuth clients are inferred from channel requirements. Each redirect URL follows the pattern `https://{domain}/oauth/{provider}/callback/{tenant}/{environment}`.

## Runner & Messaging Insights

- The deployment plan includes binding hints per runner (e.g. NATS connectivity, channel ingress) plus the WASI world name for every component so deployment packs know what to host.
- `MessagingPlan` captures the JetStream-enabled cluster topology and subjects that deployment packs may reference when producing IaC snippets.

## Example packs

### `examples/acme-pack`

- Minimal single-flow *application pack* with two secrets and two OAuth clients surfaced via annotations.
- Component manifests (`components/qa/process/manifest.json`) drive secret discovery so deployment packs render the right vault references.
- Running the CLI drops `master.tf`, `variables.tf`, and `plan.json` under `deploy/<provider>/acme/staging/`.
- `annotations.greentic.deployment` sets the preferred deployment strategy (e.g. `"iac-only"`); when omitted, the CLI `--strategy` flag is used.

### `examples/acme-plus-pack`

- Multi-flow application pack with two components (`support.automator`, `ops.router`), four secrets, two channel connectors, and explicit messaging subjects.
- Exercising this pack produces richer IaC under `deploy/<provider>/acmeplus/staging/`.
- Also declares `greentic.deployment.strategy = "iac-only"` so downstream tooling knows which deployment pack/flow to target by default.

Both packs log telemetry via `greentic-telemetry` so plan/apply/destroy traces show up in OTLP backends.

## Terraform & OpenTofu

- Provider artifacts live under `deploy/<provider>/<tenant>/<environment>/` and the CLI runs the selected IaC tool inside that directory.
- `--iac-tool` or `GREENTIC_IAC_TOOL` accept `tf|terraform|tofu|opentofu`; when unset the deployer prefers `tofu` then falls back to `terraform`.
- Apply runs `init`, `plan`, `apply plan.tfplan`; destroy runs `init`, `destroy`. `--dry-run` prints the command list without executing anything.

## Re-running provider artifacts

Once artifacts exist and secrets are stored you can re-run them manually:

- Inspect `apply-manifest.json` / `destroy-manifest.json` to confirm secret paths and OAuth redirect URLs.
- AWS: `cd deploy/aws/<tenant>/<environment>` and run the recorded Terraform/OpenTofu commands (`master.tf`, `variables.tf`, `plan.json`).
- Azure: `master.bicep` plus `parameters.json` feed `az deployment group create ...`.
- GCP: `master.yaml` plus `parameters.yaml` feed `gcloud deployment-manager deployments create ...`.
- `--dry-run` or `--preview` show the IaC shell commands without touching cloud resources.

## Try the sample packs

1. Minimal pack:
   ```bash
   cargo run -p greentic-deployer -- plan --provider aws --tenant acme --environment staging --pack examples/acme-pack
   ```
2. Inspect `deploy/aws/acme/staging/` (and matching `azure`/`gcp`) for:
   - `master.tf`, `variables.tf`, `plan.json` (AWS).
   - `master.bicep`, `parameters.json`, `plan.json` (Azure).
   - `master.yaml`, `parameters.yaml`, `plan.json` (GCP).
3. `apply`/`destroy` write manifests listing secrets, OAuth redirects, and telemetry attributes, so you can double-check before running IaC directly.
4. Repeat with the larger pack (same deployment pack, richer plan):
   ```bash
   cargo run -p greentic-deployer -- plan --provider aws --tenant acmeplus --environment staging --pack examples/acme-plus-pack
   ```

## CI smoke test

- `scripts/ci-smoke.sh` iterates over providers (`aws/azure/gcp`), actions (`apply/destroy`), and both packs in `--dry-run` mode to guarantee IaC command generation works. Local/K8s are not covered by the legacy shims and require a deployment pack executor.
- `./ci/local_check.sh` is the local equivalent run before pushing (fmt, clippy, tests, docs, and the smoke script).

## Repo settings

- Enable GitHub’s “Allow auto-merge” so Dependabot PRs can merge after required checks pass; set required status checks via branch protection as needed.

## Sample IaC output / deployment packs

- Deployment packs such as `greentic.deploy.aws` or `greentic.deploy.generic` consume the plan via `greentic:deploy-plan@1.0.0` and emit IaC under `deploy/<provider>/<tenant>/<environment>/`.
- `deploy/aws/acmeplus/staging/master.tf` highlights the ECS setup generated by the AWS deployment pack.
- `deploy/azure/acmeplus/staging/master.bicep` includes container apps and secret bindings generated by the Azure deployment pack.
- `deploy/gcp/acmeplus/staging/master.yaml` expresses Deployment Manager resources with inline Secret Manager references from the GCP deployment pack.
- See `docs/provider-visual-guide.md` (and the SVG mocks under `docs/images/`) for diagrams + screenshot tips.
- See `docs/platform_bootstrap.md` for platform bootstrap/installer architecture and manifest `bootstrap` block conventions.
- CLI scaffold includes `platform install|upgrade|status` commands (currently reporting metadata only) to support bootstrap flows; help text notes offline-first/bootstrap-state intent and verification flags.

## Adding new deployment targets

- Author a **deployment pack** (`kind: deployment`) with `type: events` flows made of deployment components (`supports: ["events"]`, `world: "greentic:deploy-plan@1.0.0"`, `host.iac.write_templates = true`).
- Publish that pack (e.g. `greentic.deploy.mycloud` with flow `deploy_mycloud_iac`).
- Update the provider/strategy mapping so `("mycloud","iac") -> ("greentic.deploy.mycloud","deploy_mycloud_iac")`.
- `greentic-deployer` remains provider-agnostic: it loads the application pack, builds the `DeploymentPlan`, selects the deployment pack based on `--provider/--strategy`, and delegates IaC generation to the flow via `greentic-runner`.
- Hosts (runner/control planes) should register their executor via:
  ```rust
  use std::sync::Arc;
  use greentic_deployer::deployment::{self, DeploymentDispatch, DeploymentExecutor};

  struct RunnerExecutor;
  #[async_trait::async_trait]
  impl DeploymentExecutor for RunnerExecutor {
      async fn execute(
          &self,
          config: &greentic_deployer::DeployerConfig,
          plan: &greentic_deployer::plan::PlanContext,
          dispatch: &DeploymentDispatch,
      ) -> greentic_deployer::Result<()> {
          // Call greentic-runner with (dispatch.pack_id, dispatch.flow_id) + plan JSON.
          Ok(())
      }
  }

  deployment::set_deployment_executor(Arc::new(RunnerExecutor));
  ```
  Registered executors receive the resolved `(pack_id, flow_id)` and can invoke `greentic-runner` using the shared bindings from `greentic-interfaces-host`. If no executor is registered the legacy Rust shims run as a fallback.
