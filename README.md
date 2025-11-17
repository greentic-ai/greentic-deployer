# Greentic Deployer

`greentic-deployer` is a CLI and library that builds cloud-agnostic deployment plans for Greentic packs and materialises provider-specific artifacts for AWS, Azure, and GCP.

## Concepts

- **Packs** describe flows, components, tools, secrets, and tenant bindings. The deployer introspects packs to understand runners, messaging, channels, secrets, OAuth, and telemetry requirements.
- **DeploymentPlan** is a cloud-agnostic model that captures messaging (NATS) topology, runner services, channel ingress, secrets, OAuth redirect URLs, and telemetry hooks.
- **Providers** map the plan to provider-specific artifacts (Terraform, Bicep, Deployment Manager snippets) and manage secrets via the configured secrets backend.

## Building

```bash
cargo build -p greentic-deployer
```

## CLI

```
greentic-deployer <plan|apply|destroy> --provider <aws|azure|gcp> \
  --tenant <tenant-id> --environment <env> --pack <path> \
  [--yes] [--preview] [--dry-run] [--iac-tool <tf|terraform|tofu|opentofu>]
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

Plans and provider artifacts are written to `deploy/<provider>/<tenant>/<environment>/` for inspection.

## Configuration

- `GREENTIC_ENV` sets the default environment (defaults to `dev`).
- `GREENTIC_BASE_DOMAIN` controls the base domain used when emitting OAuth redirect URLs (defaults to `deploy.greentic.ai`).
- OTLP tracing is wired via `GREENTIC_OTLP_ENDPOINT` or standard `OTEL_EXPORTER_OTLP_ENDPOINT`.
- `GREENTIC_IAC_TOOL` overrides the IaC tool used when running `apply`/`destroy`. Accepts `tf`/`terraform` or `tofu`/`opentofu`. When unset the deployer prefers `tofu` (if available), falls back to `terraform`, and execution fails later if the binary is absent.

## Secrets & OAuth

- Secrets are surfaced in plans with logical names (e.g. `SLACK_BOT_TOKEN`, `TEAMS_CLIENT_SECRET`) and are only fetched during `apply`/`destroy`.
- `greentic-deployer` now uses `greentic-secrets`’ `DefaultResolver` so AWS/Azure/GCP backends are auto-discovered via environment metadata and a `SecretsCore` is built for the configured tenant/environment. Apply/destroy fail fast when a required secret is missing, with a clear tenant/environment error.
- OAuth clients use `greentic-oauth`’s `ProviderId` identifiers (e.g. `google`, `microsoft`, `github`) so downstream tooling can reuse the same descriptors when wiring the broker, and redirect URLs continue to follow the `https://{domain}/oauth/{provider}/callback/{tenant}/{environment}` pattern.

-## Telemetry & Provider Artifacts

- Telemetry is instrumented via `greentic-telemetry`, which publishes OTLP spans for each `plan`, `apply`, or `destroy` action and injects a task-local `TelemetryCtx` capturing tenant/provider/session keys.
- Provider artifacts now embed the telemetry endpoint and context in the generated shell/HashiCorp/Deployment Manager snippets (for example, Terraform output includes `OTEL_EXPORTER_OTLP_ENDPOINT`, Azure Bicep adds the value under container `env`, and GCP config adds the annotation metadata), so every generated service inherits the tenant context.
- Secrets, OAuth redirects, and binding hints are surfaced directly inside the provider outputs so you can see which vault entries and redirect URLs will be consumed up front.
- OAuth clients are inferred from channel requirements. Each redirect URL follows the pattern `https://{domain}/oauth/{provider}/callback/{tenant}/{environment}`.

## Runner & Messaging Insights

- The deployment plan includes binding hints per runner (e.g. NATS connectivity, channel ingress) plus the WASI world name for every component so provider backends know what to host.
- `MessagingPlan` captures the JetStream-enabled cluster topology (cluster name, replicas, admin URL, subjects, and stream hints) that every provider artifact references in the generated Terraform/Bicep/YAML snippets.

## Example packs

### `examples/acme-pack`

- Minimal single-flow pack with two secrets and two OAuth clients surfaced via annotations.
- Component manifests (`components/qa/process/manifest.json`) drive secret discovery so provider manifests render the right vault references.
- Running the CLI drops `master.tf`, `variables.tf`, and `plan.json` under `deploy/<provider>/acme/staging/`.

### `examples/acme-plus-pack`

- Multi-flow pack with two components (`support.automator`, `ops.router`), four secrets, two channel connectors, and explicit messaging subjects.
- Exercising this pack produces richer IaC (multiple runners, channel ingress comments, expanded telemetry hints) under `deploy/<provider>/acmeplus/staging/`.

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
4. Repeat with the larger pack:
   ```bash
   cargo run -p greentic-deployer -- plan --provider aws --tenant acmeplus --environment staging --pack examples/acme-plus-pack
   ```

## CI smoke test

- `scripts/ci-smoke.sh` iterates over providers (`aws/azure/gcp`), actions (`apply/destroy`), and both packs in `--dry-run` mode to guarantee IaC command generation works.
- `./ci/local_check.sh` is the local equivalent run before pushing (fmt, clippy, tests, docs, and the smoke script).

## Sample IaC output

- `deploy/aws/acmeplus/staging/master.tf` shows ECS clusters, task definitions, runner env vars, and the greentic secret data sources.
- `deploy/azure/acmeplus/staging/master.bicep` includes container apps with `secretPaths` wiring and telemetry env vars.
- `deploy/gcp/acmeplus/staging/master.yaml` expresses Deployment Manager deployments with inline Secret Manager refs.
- See `docs/provider-visual-guide.md` (and the SVG mocks under `docs/images/`) for diagrams + screenshot tips.

## Next steps

1. Add additional pack fixtures with more flows/components to stretch runner sizing logic.
2. Capture live Terraform/Bicep/YAML snippets (beyond the mocks) once CI smoke tests run in hosted agents.
