# Provider onboarding

Use `greentic-deployer provider onboard` to install/enable provider packs that ship a `greentic.ext.provider` extension. Onboarding is schema-driven and validated by the provider-core runtime.

## Workflow
- Read the provider extension (`extensions["greentic.ext.provider"]`) from the pack manifest. When multiple providers are present, select one with `--provider-type`.
- Resolve the provider config schema from pack-local artifacts (`config_schema_ref`). Remote schemas are rejected in `--strict` mode.
- Collect configuration:
  - Interactive prompts render required fields, enums, and defaults. Fields marked with `format: "password"` or `x-secret: true` are masked.
  - Non-interactive mode accepts `--config path/to/config.json`.
- Validation:
  - JSON Schema validation via the embedded schema.
  - Semantic validation by invoking the provider runtime component export (from `runtime.export`) and calling `validate-config`.
- Persist the onboarded provider under the state directory (default `greentic.paths.state_dir/providers/<provider_type>/<instance_id>.json`), capturing the pack reference, runtime binding, and supplied config.

## CLI
```
greentic-deployer provider onboard \
  --pack path/to/provider.gtpack \
  [--provider-type vendor.secrets] \
  [--config path/to/config.json] \
  [--strict] \
  [--config-out /path/to/provider.json] \
  [--instance-id my-provider] \
  [--state-dir /tmp/greentic-state]
```

Flags:
- `--strict`: require local schemas/digests for remote extension payloads.
- `--provider-type`: required when the extension declares more than one provider.
- `--config`: skips prompts and reads config JSON from disk.
- `--config-out`: explicit output path for the persisted provider config.
- `--instance-id`: custom identifier for the persisted config (defaults to provider_type).
- `--state-dir`: override where the persisted config is written.

Remote fetches follow `greentic.paths.network` settings and are blocked when `connection=Offline` unless `--allow-remote-in-offline` is set.
