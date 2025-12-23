# Platform Bootstrap Example

This example shows a reference platform pack and flows suitable for local testing, CI, or as a starting point for real platform packs.

## Files

- `fixtures/platform-pack/pack.yaml` — pack manifest with bootstrap block (install/upgrade flows + installer component).
- `fixtures/platform-pack/flows/platform_install.ygtc` — minimal bootstrap install flow.
- `fixtures/platform-pack/flows/platform_upgrade.ygtc` — minimal bootstrap upgrade flow.
- `fixtures/platform-pack/installer.wasm` — stub installer component payload.

## CLI examples

### Interactive (CLI prompts)

```bash
cargo run -- platform install \
  --pack fixtures/platform-pack/platform.gtpack \
  --interaction cli
```

### Non-interactive (JSON answers)

```bash
cat > /tmp/answers.json <<'JSON'
{ "region": "eu-west-1" }
JSON

cargo run -- platform install \
  --pack fixtures/platform-pack/platform.gtpack \
  --interaction json \
  --answers /tmp/answers.json \
  --output /tmp/bootstrap_output.json
```

### Air-gapped workflow

1. Copy `fixtures/platform-pack/` to the target machine (no network required).
2. Run `cargo run -- platform install --pack fixtures/platform-pack/platform.gtpack --interaction json --answers /tmp/answers.json`.
3. Inspect `/tmp/bootstrap_output.json` and the bootstrap state file to confirm readiness.
