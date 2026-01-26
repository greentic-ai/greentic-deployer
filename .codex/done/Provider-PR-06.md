# PR-06.md (greentic-deployer)
# Title: Schema-driven provider onboarding + provider-core validate-config

## Goal
Install/enable providers (messaging/secrets/events) via:
- provider extension metadata
- JSON schema-driven prompts
- semantic validation via provider-core `validate-config`

## Tasks
1. **Resolve provider entries**
   - Read `PackManifest.extensions["greentic.ext.provider"]`
   - Support selecting a provider by `provider_type` or default if one provider in pack.
2. **Load JSON schema**
   - Resolve `config_schema_ref` from pack-local artifacts (preferred).
   - If remote location, require digest pinned in strict mode.
3. **Prompt engine**
   - Interactive CLI: render required fields, enums, defaults
   - Non-interactive: `--config path/to/config.json`
   - Mask secret fields (by schema annotation `format: "password"` or `x-secret: true`)
4. **Validation**
   - Validate against JSON schema (use your existing schema tooling)
   - Instantiate provider-core runtime component (Wasm) and call `validate-config`
5. **Persist configuration**
   - Store config into your config/state system (likely state-store or config manager)
   - Record provider instance identifier and pack reference
6. **Docs**
   - `docs/provider_onboarding.md`
7. **Tests**
   - Unit test: schema prompt rendering for a fixture schema
   - Integration test: onboard dummy provider-core pack (dry-run)

## Acceptance criteria
- Onboarding works for any provider_type.
- Validation is deterministic and catches semantic errors.
- No domain knowledge embedded.
