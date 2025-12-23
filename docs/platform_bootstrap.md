# Greentic Platform Bootstrap & Installer Architecture

*(gtpack-based, air-gapped-friendly, multi-interaction)*

---

## 1. Goal & Non-Goals

### Goal

Enable Greentic itself to be installed and upgraded as a **platform using gtpack**, via `greentic-deployer`, in a way that:

- Works in **air-gapped environments**
- Supports **interactive configuration & secrets collection**
- Is **deterministic, versioned, and reproducible**
- Does **not require the platform to exist before it can be installed**
- Scales to **future interaction models** (IoT, MQTT, etc.)

### Non-Goals

- No hard dependency on GUI, webserver, or LLM during bootstrap
- No implicit network access unless explicitly enabled
- No bespoke installer logic per environment in deployer
- No duplication of runtime deployment logic

---

## 2. Core Principles

### 1. Everything is a gtpack

Business applications, components, and the **Greentic platform itself** are packaged as gtpacks.

### 2. Bootstrap ≠ Runtime

Installation and upgrade flows are **explicitly separated** from normal runtime deployment.

Bootstrap flows run **before the platform exists**.

### 3. Installer logic is declarative + executable

- Declarative flow logic lives in `.ygtc`
- Optional procedural logic lives in `installer.wasm`

### 4. Host controls policy, installer controls UX

- The **installer** advertises what interaction modes it supports
- The **deployer (host)** decides what is allowed or exposed

### 5. Offline-first

- No network access is required to install from a local `.gtpack`
- Network access must be explicitly enabled and policy-gated

---

## 3. Platform Pack Structure

A platform gtpack is a **self-contained bundle**:

```text
greentic-platform-<version>.gtpack
├── pack.yaml
├── components/
│   ├── greentic-store.wasm
│   ├── greentic-runner.wasm
│   ├── greentic-nats.wasm
│   ├── installer.wasm
│   └── ...
├── flows/
│   ├── platform_install.ygtc
│   ├── platform_upgrade.ygtc
│   ├── default.ygtc
│   └── custom.ygtc
├── signatures/
│   └── attestations.json
└── metadata/
```

### Key Convention

- `platform_install.ygtc` → first-time bootstrap
- `platform_upgrade.ygtc` → upgrades
- `default.ygtc` → normal runtime deployment
- `custom.ygtc` → operator overrides

---

## 4. greentic-deployer Responsibilities

`greentic-deployer` remains **small, stable, and always present**.

### New Top-Level Commands

```bash
greentic-deployer platform install --pack <path.gtpack>
greentic-deployer platform upgrade --pack <path.gtpack>
greentic-deployer platform status
```

### Deployer Responsibilities

- Load `.gtpack` from:
  - local file / USB
  - OCI registry (future / optional)
- Verify signatures & digests **offline**
- Load and execute `platform_install.ygtc` or `platform_upgrade.ygtc`
- Provide a **minimal WASM execution host**
- Persist **bootstrap state**
- Apply deployment plans (reusing existing logic)

### Deployer Does NOT

- Hardcode install questions
- Contain environment-specific logic
- Know how secrets/config are collected

---

## 5. Bootstrap State

Bootstrap state exists **outside the platform**.

### Examples

- Local: `/var/lib/greentic/bootstrap/state.json`
- Kubernetes: ConfigMap / Secret in `greentic-system`

### Stored Data

- Installed platform version + digest
- Install timestamp
- Environment kind (local / k8s / edge)
- Last successful upgrade
- Rollback reference

This state must be readable **before the platform exists**.

---

## 6. Installer Flow Model (`platform_install.ygtc`)

### Purpose

A deterministic flow that:

1. Gathers required configuration & secrets
2. Validates them
3. Emits install outputs
4. Signals readiness to deploy the platform

### Allowed Actions

- Prompt for inputs (via installer component)
- Validate answers
- Produce:
  - Config patch (non-secret)
  - Secrets write plan
  - Warnings / required actions

### Output Contract

```json
{
  "config_patch": {},
  "secrets_writes": [
    {
      "key": "...",
      "value": "...",
      "scope": "...",
      "metadata": {}
    }
  ],
  "warnings": [],
  "ready": true
}
```

---

## 7. installer.wasm Component

### Role

`installer.wasm` is an **interaction & validation engine**, not the deployer.

It:

- Defines what interaction modes it supports
- Knows which questions to ask
- Validates and normalizes answers
- Maps answers → config + secrets

It does NOT:

- Apply deployments
- Manage infrastructure
- Decide policy

---

## 8. Interaction Modes (Extensible)

### Examples

- CLI (interactive prompts)
- JSON (non-interactive automation)
- HTTP (optional, future)
- MQTT (IoT / edge provisioning)

### Key Design Rule

> The installer advertises support; the host enforces policy.

---

## 9. Capability Negotiation

### Host (deployer) exposes

- Available interaction adapters
- Allowed transports
- Security policy (e.g. “no listeners”, “offline only”)

### Installer behavior

1. Query host capabilities
2. Choose best supported mode
3. Fall back gracefully

### Example

**Air-gapped server**
- CLI ✅
- JSON ✅
- HTTP ❌
- MQTT ❌

**IoT device**
- MQTT ✅
- CLI ❌

---

## 10. Secrets Handling (Bootstrap-Safe)

Secrets are **never embedded in config**.

### Supported Backends

- Local encrypted file (default for air-gapped)
- Kubernetes Secrets
- HSM / future backends

### Execution Model

Installer emits **intent**.  
Deployer performs writes via:

```
greentic:secrets-store
```

---

## 11. Upgrade Flow (`platform_upgrade.ygtc`)

Upgrade flow may include:

- Version compatibility checks
- Schema migrations
- Rolling / blue-green hints
- Rollback references

Upgrade logic is **versioned with the platform pack**.

---

## 12. Air-Gapped Workflow (End-to-End)

```bash
# USB contains greentic-platform-0.4.0.gtpack
greentic-deployer platform install   --pack ./greentic-platform-0.4.0.gtpack   --wizard   --secrets-backend file:/var/lib/greentic/secrets.db
```

No network required.

---

## 13. IoT / Edge Future Flow (Illustrative)

1. Device boots with deployer + platform gtpack
2. Installer selects MQTT mode
3. Publishes schema to:
   ```text
   greentic/install/<device-id>/schema
   ```
4. Commissioning tool replies with answers
5. Installer validates → deployer applies

No change to deployer core required.

---

## 14. Manifest-Level Convention

In `pack.yaml`:

```yaml
bootstrap:
  install_flow: platform_install
  upgrade_flow: platform_upgrade
  installer_component: installer
```

This avoids filename guessing.

---

## 15. Implementation Phasing

### Phase 1 – Deployer
- `.gtpack` local resolver
- `platform install / upgrade`
- bootstrap state persistence

### Phase 2 – Flow Execution
- Load `platform_install.ygtc`
- Execute flows pre-platform

### Phase 3 – Installer WASM
- Minimal host runtime
- CLI + JSON adapters
- Capability negotiation

### Phase 4 – Secrets Integration
- Bootstrap secrets backend
- Secrets write execution

### Phase 5 – Upgrade Safety
- Rollback support
- Upgrade preflight checks
