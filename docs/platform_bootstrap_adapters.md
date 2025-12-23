# Platform Bootstrap Adapters

This guide describes how the bootstrap installer should interact with the host across adapters and which policy flags govern each transport. It complements `docs/platform_bootstrap.md` and the reference flows under `fixtures/platform-adapters/`.

## Policy Defaults and Flags
- `--interaction` (`auto|cli|json|http|mqtt`): installer-selected mode is constrained by host policy.
- `--allow-network` (default: false): required for HTTP/MQTT adapters; blocked when `--offline-only` is set.
- `--allow-listeners` (default: false): required for HTTP/MQTT because they open a local listener or broker connection.
- `--net-allowlist <csv>`: required when network is allowed; hosts/brokers/endpoints must match allowlist entries (domains or CIDRs).
- `--offline-only` (default: false): hard blocks outbound calls; OCI fetch and HTTP/MQTT adapters are disabled.

## Adapter Behavior

### CLI
- Interactive prompts on stdout/stdin.
- Defaults shown inline; empty input without a default fails the flow.
- No network or listener requirements.

### JSON
- Non-interactive; `--answers <path|@->` must be provided.
- Answers object keys must match question `id`s; missing keys fall back to defaults when present, otherwise fail.
- No network or listener requirements.

### HTTP
- Requires `--allow-listeners` and `--allow-network`; rejected when `--offline-only` is set.
- Listens on `--bind` (default `127.0.0.1:0`), printing the bound address.
- Endpoints:
  - `GET /schema` → `{"questions":[{id,prompt,default?}, ...]}`
  - `POST /answers` with JSON body containing an object keyed by question ids.
- Timeouts controlled by `--interaction-timeout` (seconds).
- Reference flow: `fixtures/platform-adapters/http_endpoints.ygtc`.

### MQTT
- Requires `--allow-listeners` and `--allow-network`; rejected when `--offline-only` is set.
- Enforces broker host against `--net-allowlist`.
- Topics (prefix defaults to `greentic/bootstrap` in fixtures):
  - Schema: `<prefix>/<device_id>/schema`
  - Answers: `<prefix>/<device_id>/answers`
  - Status updates: `<prefix>/<device_id>/status` (optional)
- Payloads are JSON; answers object keys mirror question `id`s.
- Reference flow: `fixtures/platform-adapters/mqtt_schema_publish.ygtc`.

## Reference Flows and Fixtures
- Multi-step CLI/JSON wizard: `fixtures/platform-adapters/multi_step_wizard.ygtc`
  - Two prompts with defaults, followed by an installer output containing config patch and secrets.
- HTTP endpoints example: `fixtures/platform-adapters/http_endpoints.ygtc`
  - Documents schema/answers endpoints and ready output.
- MQTT schema publish example: `fixtures/platform-adapters/mqtt_schema_publish.ygtc`
  - Describes topics and includes a ready output.

Tests load these fixtures as smoke coverage to ensure the files stay parseable; see `tests/platform_adapter_fixtures.rs`.
