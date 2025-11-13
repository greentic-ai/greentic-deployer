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

- The deployment plan now includes binding hints per runner (e.g. NATS connectivity, channel ingress) plus the WASI world name for every component so provider backends know what to host.
- `MessagingPlan` captures the JetStream-enabled cluster topology (cluster name, replicas, admin URL, subjects, and stream hints) that every provider artifact currently references in the generated Terraform/Bicep/YAML snippets.

## Example Pack

See `examples/acme-pack` for a minimal pack that declares a messaging flow and exposes secrets and OAuth annotations. Running the CLI against it produces:

- A normalized `DeploymentPlan` describing NATS subjects, runners, channels, secrets, and telemetry.
- Provider artifacts (Terraform HCL, Azure Bicep, GCP Deployment Manager YAML) ready to be committed or applied.
- OAuth redirect URLs inside the plan output for manual registration with Slack/Teams.

```
deploy/aws/acme/staging/main.tf
deploy/aws/acme/staging/plan.json
```

The plan also logs telemetry via `greentic-telemetry` so operations are traceable across plan/apply/destroy.

## Terraform & OpenTofu

- `greentic-deployer` writes provider artifacts under `deploy/<provider>/<tenant>/<environment>/` and then runs the chosen IaC tool inside that directory.
- The CLI accepts `--iac-tool tf|terraform` or `--iac-tool tofu|opentofu`, or you can set `GREENTIC_IAC_TOOL`. When neither flag nor env var is provided the deployer tries to auto-detect by looking for `tofu` first, then `terraform`; if neither binary exists the commands will fail later with a clear error describing the missing tool.
- Apply runs: `tool init -input=false`, `tool plan -input=false -out=plan.tfplan`, `tool apply -input=false -auto-approve plan.tfplan`. Destroy runs: `tool init -input=false` then `tool destroy -input=false -auto-approve`.
- Use `--dry-run` to print the commands that would run without executing them (this also skips the secret push/apply/destroy cycles). The commands are also logged whenever `--preview` is used.
- Apply/destroy still rely on user-provided cloud credentials and backend configuration; we report failures faithfully when the tool exits non-zero.

## Try the acme pack

1. Generate a plan against the sample pack:
   ```bash
   cargo run -p greentic-deployer -- plan --provider aws --tenant acme --environment staging --pack examples/acme-pack
   ```
2. Inspect `deploy/aws/acme/staging/` (and the matching `azure`/`gcp` roots) for:
   - `main.tf`, `variables.tf`, `plan.json` (AWS).
   - `main.bicep`, `parameters.json`, `plan.json` (Azure).
   - `main.yaml`, `parameters.yaml`, `plan.json` (GCP).
3. After running `apply`/`destroy`, check `apply-manifest.json`/`destroy-manifest.json` to see the secrets, OAuth redirect URLs, telemetry attributes, and provider targets that were recorded for that action in each vendor directory (apply now also pushes the resolved secrets into AWS Secrets Manager/Azure Key Vault/GCP Secret Manager via `greentic-secrets`).
4. Each generated file embeds NATS/runner bindings, telemetry env vars, and annotated secrets/OAuth URLs so you can review before applying.
5. To apply the infrastructure you can run `terraform init && terraform apply` under that directory (or hydrate the Bicep/YAML with your own deploy tooling) after wiring the secret identifiers via `greentic-secrets`. Run `greentic-deployer apply`/`destroy` with `--dry-run` or `--preview` to print the exact Terraform/OpenTofu commands without touching cloud resources.

## Next Steps

1. Expand the end-to-end tests to cover the full apply/destroy cycle per provider so we can verify secrets, OAuth, telemetry, and runners live beyond the mock runner.
2. Document how to re-run the generated Terraform/Bicep/YAML artifacts in the cloud once IaC commands succeed and secrets are stored.
3. Add a provider-level smoke test that runs `greentic-deployer apply`/`destroy` with `--dry-run` inside CI to ensure `iac-tool` detection and command generation keeps working.
