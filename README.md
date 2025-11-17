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
  --tenant <tenant-id> --environment <env> --pack <path> [--yes] [--preview]
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

<<<<<<< Updated upstream
```
deploy/aws/acme/staging/main.tf
deploy/aws/acme/staging/plan.json
=======
### `examples/acme-plus-pack`

- Multi-flow pack with two components (`support.automator`, `ops.router`), four secrets, and two channel connectors.
- `meta.annotations.connectors` declares messaging subjects plus Slack/Teams ingress, so the plan includes channel entries and messaging topology.
- Useful for testing larger manifests: IaC artifacts are emitted under `deploy/<provider>/acmeplus/staging/`.

Both packs log telemetry via `greentic-telemetry`, so operations are traceable across plan/apply/destroy.

## Terraform & OpenTofu

- `greentic-deployer` writes provider artifacts under `deploy/<provider>/<tenant>/<environment>/` and then runs the chosen IaC tool inside that directory.
- The CLI accepts `--iac-tool tf|terraform` or `--iac-tool tofu|opentofu`, or you can set `GREENTIC_IAC_TOOL`. When neither flag nor env var is provided the deployer tries to auto-detect by looking for `tofu` first, then `terraform`; if neither binary exists the commands will fail later with a clear error describing the missing tool.
- Apply runs: `tool init -input=false`, `tool plan -input=false -out=plan.tfplan`, `tool apply -input=false -auto-approve plan.tfplan`. Destroy runs: `tool init -input=false` then `tool destroy -input=false -auto-approve`.
- Use `--dry-run` to print the commands that would run without executing them (this also skips the secret push/apply/destroy cycles). The commands are also logged whenever `--preview` is used.
- Apply/destroy still rely on user-provided cloud credentials and backend configuration; we report failures faithfully when the tool exits non-zero.

## Re-running provider artifacts

Once the CLI has written artifacts and your secrets live in the configured vault, you can re-run the generated IaC manually:

- Inspect `deploy/<provider>/<tenant>/<environment>/apply-manifest.json` to double-check which secret identifiers and OAuth redirect URLs are expected before kicking off your own runs.
- AWS: `cd deploy/aws/<tenant>/<environment>` and run the same commands printed during apply (for example `terraform init`, `terraform plan`, `terraform apply` or switch to `tofu` if you prefer OpenTofu). The directory contains `master.tf`, `variables.tf`, and the serialized `plan.json`.
- Azure: use `master.bicep` + `parameters.json` with `az deployment group create --resource-group <rg> --template-file master.bicep --parameters @parameters.json`. The `secretPaths` parameter already maps each logical secret name to the vault path emitted in the manifest.
- GCP: feed `master.yaml` and `parameters.yaml` to `gcloud deployment-manager deployments create ... --config master.yaml --properties=properties.yaml` (or your preferred DM workflow). Telemetry annotations and secret hints are embedded so you can audit everything before running.
- Destroy follows the same pattern—invoke the appropriate IaC destroy command from the provider directory or run `greentic-deployer destroy --dry-run` first to see the exact commands that would execute.

## Try the sample packs

1. Generate a plan against the minimal pack:
   ```bash
   cargo run -p greentic-deployer -- plan --provider aws --tenant acme --environment staging --pack examples/acme-pack
   ```
2. Inspect `deploy/aws/acme/staging/` (and the matching `azure`/`gcp` roots) for:
   - `master.tf`, `variables.tf`, `plan.json` (AWS).
   - `master.bicep`, `parameters.json`, `plan.json` (Azure).
   - `master.yaml`, `parameters.yaml`, `plan.json` (GCP).
3. After running `apply`/`destroy`, check `apply-manifest.json`/`destroy-manifest.json` to see the secrets, OAuth redirect URLs, telemetry attributes, and provider targets that were recorded for that action in each vendor directory (apply now also pushes the resolved secrets into AWS Secrets Manager/Azure Key Vault/GCP Secret Manager via `greentic-secrets`).
4. Each generated file embeds NATS/runner bindings, telemetry env vars, and annotated secrets/OAuth URLs so you can review before applying.
5. To apply the infrastructure you can run `terraform init && terraform apply` under that directory (or hydrate the Bicep/YAML with your own deploy tooling) after wiring the secret identifiers via `greentic-secrets`. Run `greentic-deployer apply`/`destroy` with `--dry-run` or `--preview` to print the exact Terraform/OpenTofu commands without touching cloud resources.
6. Repeat the same flow for the complex pack:
   ```bash
   cargo run -p greentic-deployer -- plan --provider aws --tenant acmeplus --environment staging --pack examples/acme-plus-pack
   ```
   This surfaces multiple channels, four secrets, and two OAuth clients in the resulting plan.

## CI smoke test

Use `scripts/ci-smoke.sh` inside CI to verify that `greentic-deployer` can still detect the IaC tool and renders dry-run commands for every provider/action combination:

```bash
./scripts/ci-smoke.sh
>>>>>>> Stashed changes
```

The plan also logs telemetry via `greentic-telemetry` so operations are traceable across plan/apply/destroy.

## Next Steps

<<<<<<< Updated upstream
1. Replace the stub provider backends with Terraform/Pulumi template generation and apply logic.
2. Wire secrets into AWS Secrets Manager, Azure Key Vault, and GCP Secret Manager during `apply`.
3. Extend introspection to hydrate runner bindings, channel-specific ingress routes, and real OAuth registration helpers.
4. Add end-to-end tests against real Greentic packs and provider mocks.
=======
Locally, run `./ci/local_check.sh` before pushing. It executes `cargo fmt`, `cargo clippy`, `cargo test`, `cargo doc`, and finally `scripts/ci-smoke.sh` so your branch mirrors the CI pipeline.

## Sample IaC output

### AWS (`examples/acme-plus-pack`)

`deploy/aws/acmeplus/staging/master.tf` highlights the ECS setup:

```hcl
terraform {
  backend "local" {
    path = "terraform.tfstate"
  }
}

locals {
  nats_cluster = "default"
  nats_admin_url = "https://nats.staging.acmeplus.svc"
  telemetry_endpoint = "https://otel.greentic.ai"
}

# Secret data sources (values resolved via greentic-secrets)
data "aws_secretsmanager_secret_version" "secret_slack_bot_token" {
  secret_id = var.slack_bot_token_secret_id
}

resource "aws_ecs_cluster" "nats" {
  name = local.nats_cluster
}
```

For each runner the file emits paired task/service blocks:

```hcl
resource "aws_ecs_task_definition" "runner_greentic-acme-plus" {
  family = "greentic-acme-plus"
  cpu    = "512"
  memory = "1024"
  requires_compatibilities = ["FARGATE"]
  container_definitions = <<EOF
[ {
  "name": "greentic-acme-plus",
  "image": "greentic/runner:latest",
  "environment": [
    { "name": "NATS_URL", "value": "https://nats.staging.acmeplus.svc" },
    { "name": "OTEL_EXPORTER_OTLP_ENDPOINT", "value": "https://otel.greentic.ai" }
  ]
} ]
EOF
}
```

`variables.tf` mirrors the secrets so you can wire them into Secrets Manager, and `plan.json` keeps the full `DeploymentPlan`.

### Azure (`examples/acme-plus-pack`)

`deploy/azure/acmeplus/staging/master.bicep` includes container apps and secret bindings:

```bicep
param tenant string = 'acmeplus'
param environment string = 'staging'
param telemetryEndpoint string = 'https://otel.greentic.ai'
param natsAdminUrl string = 'https://nats.staging.acmeplus.svc'
param secretPaths object = {}

resource runnerSupportAutomator 'Microsoft.Web/containerApps@2023-08-01' = {
  name: '${tenant}-${environment}-support'
  location: resourceGroup().location
  properties: {
    configuration: {
      secrets: [
        { name: 'SLACK_BOT_TOKEN', value: secretPaths['SLACK_BOT_TOKEN'] }
        { name: 'CRM_API_TOKEN', value: secretPaths['CRM_API_TOKEN'] }
      ]
    }
    template: {
      containers: [
        {
          name: 'support.automator'
          image: 'greentic/runner:latest'
          env: [
            { name: 'NATS_URL', value: natsAdminUrl }
            { name: 'OTEL_EXPORTER_OTLP_ENDPOINT', value: telemetryEndpoint }
          ]
        }
      ]
    }
  }
}
```

`parameters.json` maps `secretPaths` so you can point each logical secret at its Key Vault identifier.

### GCP (`examples/acme-plus-pack`)

`deploy/gcp/acmeplus/staging/master.yaml` expresses Deployment Manager resources calling out secrets and telemetry annotations. For example:

```yaml
resources:
  - name: greentic-acmeplus-runner
    type: container.v1beta1.deployment
    properties:
      containers:
        - name: greentic-acmeplus
          image: gcr.io/greentic/runner:latest
          env:
            - name: NATS_URL
              value: https://nats.staging.acmeplus.svc
            - name: OTEL_EXPORTER_OTLP_ENDPOINT
              value: https://otel.greentic.ai
            - name: SLACK_BOT_TOKEN
              valueFrom:
                secretKeyRef:
                  name: projects/greentic/secrets/greentic-acmeplus-slack_bot_token
                  key: latest
```

`parameters.yaml` mirrors the secret references and runner sizing knobs.

These snippets are all generated under `deploy/<provider>/<tenant>/<environment>/` so you can copy/paste or adapt them directly in your IaC workflows. For visual walkthroughs (diagrams + screenshot tips), see `docs/provider-visual-guide.md`.
>>>>>>> Stashed changes
