# Platform Bootstrap Glossary

- **gtpack**: Greentic package format bundling manifests, flows, components, signatures, and metadata for distribution and offline install/upgrade.
- **platform pack**: A gtpack that contains the Greentic platform itself (services, installer, flows) and conventions for install/upgrade (`platform_install.ygtc`, `platform_upgrade.ygtc`).
- **bootstrap vs runtime**: Bootstrap flows run before the platform exists to collect config/secrets and prep deployment; runtime flows are the normal application/platform deployments after bootstrap is complete.
- **platform_install.ygtc / platform_upgrade.ygtc**: Declarative flow files inside the platform pack; the former handles first-time install, the latter handles upgrades (version checks, migrations, rollback hints).
- **installer.wasm**: Optional procedural component embedded in the platform pack that drives interaction/validation, advertises supported modes, and emits config/secrets intents; it does not apply deployments.
- **capability negotiation**: The host (deployer) exposes allowed transports/adapters; the installer selects a supported mode (CLI, JSON, HTTP, MQTT, etc.) and falls back if needed, honoring host policy.
- **bootstrap state**: Metadata persisted outside the platform (e.g., local file or K8s ConfigMap/Secret) capturing installed version/digest, timestamps, environment kind, last upgrade, and rollback reference—readable before the platform exists.
- **config_patch vs secrets_writes**: Installer outputs are split into non-secret configuration changes (`config_patch`) and a declarative list of secret writes (`secrets_writes`) that the deployer executes via the secrets-store backend.
