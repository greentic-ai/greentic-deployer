use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use greentic_config::ConfigResolver;
use greentic_config_types::{ConnectionKind, GreenticConfig, NetworkConfig};
use greentic_types::pack_manifest::{ExtensionRef, PackManifest};
use greentic_types::{PackId, ProviderDecl, ProviderExtensionInline};
use jsonschema::JSONSchema;
use semver::Version;
use serde::Serialize;
use serde_json::{Map as JsonMap, Value as JsonValue};
use sha2::{Digest, Sha256};
use wasmtime::{
    Config as WasmtimeConfig, Engine, Store,
    component::{Component, Linker},
};

use crate::error::{DeployerError, Result};
use crate::pack_introspect::{read_manifest_from_directory, read_manifest_from_gtpack};
use crate::path_safety::normalize_under_root;

const PROVIDER_EXTENSION_KEY: &str = "greentic.ext.provider";

#[derive(Debug)]
pub struct OnboardRequest {
    pub pack_path: PathBuf,
    pub provider_type: Option<String>,
    pub config_path: Option<PathBuf>,
    pub config_out: Option<PathBuf>,
    pub strict: bool,
    pub instance_id: Option<String>,
    pub state_dir: Option<PathBuf>,
    pub allow_remote_in_offline: bool,
    pub greentic_config: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct StoredProviderConfig {
    provider_type: String,
    instance_id: String,
    pack_id: String,
    pack_version: String,
    config_schema_ref: String,
    runtime: RuntimeSnapshot,
    config: JsonValue,
}

#[derive(Debug, Serialize)]
struct RuntimeSnapshot {
    component_ref: String,
    export: String,
    world: String,
}

#[derive(Debug)]
pub struct OnboardOutcome {
    pub provider_type: String,
    pub instance_id: String,
    pub config_path: PathBuf,
    pub pack_id: PackId,
    pub pack_version: Version,
}

enum PackLocation {
    Dir(PathBuf),
    Archive(PathBuf),
}

impl PackLocation {
    fn open(path: &Path) -> Result<Self> {
        if path.is_dir() {
            Ok(Self::Dir(path.to_path_buf()))
        } else {
            Ok(Self::Archive(path.to_path_buf()))
        }
    }

    fn read_manifest(&self) -> Result<PackManifest> {
        match self {
            Self::Dir(path) => read_manifest_from_directory(path),
            Self::Archive(path) => read_manifest_from_gtpack(path),
        }
    }

    fn read_entry(&self, relative: &Path) -> Result<Vec<u8>> {
        match self {
            Self::Dir(root) => {
                let path = normalize_under_root(root, relative)
                    .map_err(|err| DeployerError::Pack(err.to_string()))?;
                Ok(fs::read(path)?)
            }
            Self::Archive(path) => crate::pack_introspect::read_entry_from_gtpack(path, relative),
        }
    }
}

pub fn onboard(request: OnboardRequest) -> Result<OnboardOutcome> {
    let config = load_greentic_config(request.greentic_config.as_ref(), request.state_dir.clone())?;
    if matches!(config.environment.connection, Some(ConnectionKind::Offline))
        && !request.allow_remote_in_offline
    {
        return Err(DeployerError::OfflineDisallowed(
            "connection is Offline; remote fetches require --allow-remote-in-offline".into(),
        ));
    }

    let pack = PackLocation::open(&request.pack_path)?;
    let manifest = pack.read_manifest()?;

    let provider_extension = extract_provider_extension(&manifest)?;
    let inline =
        resolve_extension_payload(provider_extension, &pack, request.strict, &config.network)?;
    let provider = select_provider(&inline.providers, request.provider_type.as_deref())?;

    let schema = load_schema(
        &pack,
        &provider.config_schema_ref,
        request.strict,
        &config.network,
    )?;
    let config_value = load_or_prompt_config(&schema, request.config_path.as_deref())?;

    validate_against_schema(&schema, &config_value)?;
    validate_with_runtime(&pack, provider, &config_value)?;

    let instance_id = sanitize_instance_id(
        request
            .instance_id
            .clone()
            .unwrap_or_else(|| provider.provider_type.clone()),
    );
    let persisted_path = persist_provider_config(
        &config,
        &manifest,
        provider,
        &instance_id,
        &config_value,
        request.state_dir.as_deref(),
        request.config_out.as_deref(),
    )?;

    Ok(OnboardOutcome {
        provider_type: provider.provider_type.clone(),
        instance_id,
        config_path: persisted_path,
        pack_id: manifest.pack_id.clone(),
        pack_version: manifest.version.clone(),
    })
}

fn load_greentic_config(
    explicit_config: Option<&PathBuf>,
    override_state: Option<PathBuf>,
) -> Result<GreenticConfig> {
    let mut resolver = ConfigResolver::new();
    if let Some(layer) = load_explicit_config(explicit_config)? {
        resolver = resolver.with_cli_overrides(layer);
    }
    let mut config = resolver
        .load()
        .map_err(|err| DeployerError::Config(err.to_string()))?
        .config;
    if let Some(path) = override_state {
        config.paths.state_dir = path.clone();
        config.paths.greentic_root = path.clone();
        config.paths.cache_dir = path.join("cache");
        config.paths.logs_dir = path.join("logs");
    }
    Ok(config)
}

fn load_explicit_config(path: Option<&PathBuf>) -> Result<Option<greentic_config::ConfigLayer>> {
    let Some(path) = path else {
        return Ok(None);
    };

    let contents = fs::read_to_string(path).map_err(|err| {
        DeployerError::Config(format!(
            "failed to read config file {}: {err}",
            path.display()
        ))
    })?;

    let format = match path.extension().and_then(|s| s.to_str()) {
        Some("json") => greentic_config::ConfigFileFormat::Json,
        _ => greentic_config::ConfigFileFormat::Toml,
    };

    let layer = match format {
        greentic_config::ConfigFileFormat::Toml => {
            toml::from_str::<greentic_config::ConfigLayer>(&contents)
                .map_err(|err| format!("toml parse error: {err}"))
        }
        greentic_config::ConfigFileFormat::Json => {
            serde_json::from_str::<greentic_config::ConfigLayer>(&contents)
                .map_err(|err| format!("json parse error: {err}"))
        }
    }
    .map_err(|err| {
        DeployerError::Config(format!("invalid config file {}: {err}", path.display()))
    })?;

    Ok(Some(layer))
}

fn extract_provider_extension(manifest: &PackManifest) -> Result<&ExtensionRef> {
    let extensions = manifest
        .extensions
        .as_ref()
        .ok_or_else(|| DeployerError::Pack("pack has no extensions block".into()))?;
    extensions.get(PROVIDER_EXTENSION_KEY).ok_or_else(|| {
        DeployerError::Pack("provider extension missing (greentic.ext.provider)".into())
    })
}

fn resolve_extension_payload(
    extension: &ExtensionRef,
    pack: &PackLocation,
    strict: bool,
    network: &NetworkConfig,
) -> Result<ProviderExtensionInline> {
    if let Some(inline) = &extension.inline {
        return serde_json::from_value(inline.clone()).map_err(DeployerError::Json);
    }

    let location = extension.location.as_deref().ok_or_else(|| {
        DeployerError::Pack("provider extension missing inline payload and location".into())
    })?;
    let bytes = fetch_payload(location, pack, network)?;
    if strict && location.starts_with("http") && extension.digest.is_none() {
        return Err(DeployerError::Config(
            "strict mode requires digest for remote extension payload".into(),
        ));
    }
    if let Some(expected) = extension.digest.as_deref() {
        verify_digest(expected, &bytes)?;
    }
    let value: JsonValue = serde_json::from_slice(&bytes)?;
    serde_json::from_value(value).map_err(DeployerError::Json)
}

fn fetch_payload(location: &str, pack: &PackLocation, network: &NetworkConfig) -> Result<Vec<u8>> {
    if location.starts_with("http://") || location.starts_with("https://") {
        let client = build_http_client(network, location)?;
        let resp = client
            .get(location)
            .send()
            .map_err(|err| DeployerError::Other(err.to_string()))?
            .error_for_status()
            .map_err(|err| DeployerError::Other(err.to_string()))?;
        let mut buf = Vec::new();
        let mut reader = resp;
        reader
            .read_to_end(&mut buf)
            .map_err(|err| DeployerError::Other(err.to_string()))?;
        Ok(buf)
    } else {
        let path = location.strip_prefix("file://").unwrap_or(location);
        let rel = PathBuf::from(path);
        pack.read_entry(&rel)
    }
}

fn build_http_client(network: &NetworkConfig, base_url: &str) -> Result<reqwest::blocking::Client> {
    let mut builder = reqwest::blocking::Client::builder();

    if let Some(proxy_url) = &network.proxy_url {
        let proxy = reqwest::Proxy::all(proxy_url).map_err(|err| {
            DeployerError::Config(format!("invalid proxy URL {proxy_url}: {err}"))
        })?;
        builder = builder.proxy(proxy);
    }

    builder = match network.tls_mode {
        greentic_config_types::TlsMode::Disabled => {
            if base_url.starts_with("https://") {
                return Err(DeployerError::Config(
                    "network.tls_mode=disabled is not allowed for https URLs".into(),
                ));
            }
            builder
        }
        _ => builder,
    };

    if let Some(connect_ms) = network.connect_timeout_ms {
        builder = builder.connect_timeout(std::time::Duration::from_millis(connect_ms));
    }
    if let Some(read_ms) = network.read_timeout_ms {
        builder = builder.timeout(std::time::Duration::from_millis(read_ms));
    }

    builder.build().map_err(|err| {
        DeployerError::Config(format!(
            "failed to build HTTP client for provider extension: {err}"
        ))
    })
}

fn select_provider<'a>(
    providers: &'a [ProviderDecl],
    requested: Option<&str>,
) -> Result<&'a ProviderDecl> {
    if let Some(requested) = requested {
        providers
            .iter()
            .find(|p| p.provider_type == requested)
            .ok_or_else(|| {
                DeployerError::Config(format!(
                    "provider_type '{requested}' not found in extension"
                ))
            })
    } else if providers.len() == 1 {
        Ok(&providers[0])
    } else {
        Err(DeployerError::Config(
            "multiple providers present; select one with --provider-type".into(),
        ))
    }
}

fn load_schema(
    pack: &PackLocation,
    schema_ref: &str,
    strict: bool,
    network: &NetworkConfig,
) -> Result<JsonValue> {
    if schema_ref.starts_with("http://") || schema_ref.starts_with("https://") {
        if strict {
            return Err(DeployerError::Config(
                "strict mode requires config_schema_ref to be pack-local".into(),
            ));
        }
        let bytes = fetch_payload(schema_ref, pack, network)?;
        return serde_json::from_slice(&bytes).map_err(DeployerError::Json);
    }

    let path = schema_ref.strip_prefix("file://").unwrap_or(schema_ref);
    let schema_bytes = pack.read_entry(Path::new(path))?;
    serde_json::from_slice(&schema_bytes).map_err(DeployerError::Json)
}

fn load_or_prompt_config(schema: &JsonValue, path: Option<&Path>) -> Result<JsonValue> {
    if let Some(path) = path {
        let contents = fs::read_to_string(path)?;
        let value: JsonValue = serde_json::from_str(&contents)?;
        if !value.is_object() {
            return Err(DeployerError::Config(
                "config JSON must be an object (top-level map)".into(),
            ));
        }
        return Ok(value);
    }

    let questions = build_questions(schema)?;
    prompt_answers(&questions)
}

#[derive(Debug)]
struct Question {
    id: String,
    title: String,
    default: Option<JsonValue>,
    options: Option<Vec<JsonValue>>,
    secret: bool,
    required: bool,
}

fn build_questions(schema: &JsonValue) -> Result<Vec<Question>> {
    let obj = schema
        .as_object()
        .ok_or_else(|| DeployerError::Config("schema root must be an object".into()))?;
    let required: BTreeSet<String> = obj
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let properties = obj
        .get("properties")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            DeployerError::Config("schema must include object.properties for prompting".into())
        })?;

    let mut questions = Vec::new();
    for (name, entry) in properties {
        let entry_obj = entry.as_object().cloned().unwrap_or_default();
        let title = entry_obj
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(name)
            .to_string();
        let default = entry_obj.get("default").cloned();
        let options = entry_obj
            .get("enum")
            .and_then(|v| v.as_array().cloned())
            .map(|arr| arr.into_iter().collect());
        let secret = is_secret_field(&entry_obj);
        questions.push(Question {
            id: name.clone(),
            title,
            default,
            options,
            secret,
            required: required.contains(name),
        });
    }
    Ok(questions)
}

fn is_secret_field(entry: &JsonMap<String, JsonValue>) -> bool {
    entry
        .get("format")
        .and_then(|v| v.as_str())
        .map(|v| v == "password")
        .unwrap_or(false)
        || entry
            .get("x-secret")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

fn prompt_answers(questions: &[Question]) -> Result<JsonValue> {
    let mut answers = JsonMap::new();
    for q in questions {
        let value = loop {
            let prompt = format!(
                "{}{}{}: ",
                q.title,
                if q.required { "" } else { " (optional)" },
                q.default
                    .as_ref()
                    .map(|d| format!(
                        " [default: {}]",
                        serde_json::to_string(d).unwrap_or_else(|_| "<invalid>".into())
                    ))
                    .unwrap_or_default()
            );
            let input = if q.secret {
                rpassword::prompt_password(prompt.clone())
            } else {
                print!("{prompt}");
                io::stdout().flush().ok();
                let mut line = String::new();
                io::stdin().read_line(&mut line).map(|_| line)
            };
            match input {
                Ok(value) => match coerce_answer(value.trim(), q) {
                    Ok(v) => break v,
                    Err(err) => {
                        eprintln!("{err}");
                        continue;
                    }
                },
                Err(err) => {
                    eprintln!("failed to read input: {err}");
                    continue;
                }
            }
        };
        answers.insert(q.id.clone(), value);
    }
    Ok(JsonValue::Object(answers))
}

fn coerce_answer(input: &str, q: &Question) -> Result<JsonValue> {
    if input.is_empty() {
        if let Some(default) = &q.default {
            return Ok(default.clone());
        }
        if !q.required {
            return Ok(JsonValue::Null);
        }
        return Err(DeployerError::Config(format!(
            "no value provided for required field {}",
            q.id
        )));
    }

    let parsed =
        serde_json::from_str::<JsonValue>(input).unwrap_or(JsonValue::String(input.into()));
    if let Some(options) = &q.options
        && !options.contains(&parsed)
    {
        let opts = options
            .iter()
            .map(|o| serde_json::to_string(o).unwrap_or_else(|_| "<invalid>".into()))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(DeployerError::Config(format!(
            "value for {} must be one of: {}",
            q.id, opts
        )));
    }

    Ok(parsed)
}

fn validate_against_schema(schema: &JsonValue, config: &JsonValue) -> Result<()> {
    let compiled = JSONSchema::compile(schema)
        .map_err(|err| DeployerError::Config(format!("invalid JSON schema: {err}")))?;
    compiled.validate(config).map_err(|errors| {
        let joined = errors
            .into_iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        DeployerError::Config(format!("config failed schema validation: {joined}"))
    })
}

fn validate_with_runtime(
    pack: &PackLocation,
    provider: &ProviderDecl,
    config: &JsonValue,
) -> Result<()> {
    let mut config_builder = WasmtimeConfig::new();
    config_builder.wasm_component_model(true);
    let engine = Engine::new(&config_builder)
        .map_err(|err| DeployerError::Other(format!("wasmtime init failed: {err}")))?;
    let file_name = if provider.runtime.component_ref.ends_with(".wasm") {
        PathBuf::from(&provider.runtime.component_ref)
    } else {
        PathBuf::from(format!("{}.wasm", provider.runtime.component_ref))
    };
    let wasm_path = Path::new("components").join(file_name);
    let wasm_bytes = pack.read_entry(&wasm_path)?;
    let input_bytes = serde_json::to_vec(config)?;

    match Component::from_binary(&engine, &wasm_bytes) {
        Ok(component) => validate_component(
            &engine,
            component,
            provider,
            &input_bytes,
            &provider.runtime.export,
        ),
        Err(_) => validate_with_core_module(&engine, &wasm_bytes, &provider.runtime.export),
    }
}

fn validate_component(
    engine: &Engine,
    component: Component,
    provider: &ProviderDecl,
    input_bytes: &[u8],
    export: &str,
) -> Result<()> {
    let mut store = Store::new(engine, ());
    let linker = Linker::new(engine);
    let instance = linker.instantiate(&mut store, &component).map_err(|err| {
        DeployerError::Other(format!("failed to instantiate provider runtime: {err}"))
    })?;

    let candidates = [
        export.to_string(),
        format!("{}#{export}", provider.runtime.world),
    ];
    let func = candidates
        .iter()
        .find_map(|name| {
            instance
                .get_typed_func::<(Vec<u8>,), (Vec<u8>,)>(&mut store, name)
                .ok()
        })
        .ok_or_else(|| {
            DeployerError::Other(format!(
                "runtime export '{}' not found (tried {:?})",
                export, candidates
            ))
        })?;

    let (output,) = func
        .call(&mut store, (input_bytes.to_vec(),))
        .map_err(|err| DeployerError::Other(format!("validate-config call failed: {err}")))?;
    interpret_validation_output(&output)
}

fn validate_with_core_module(engine: &Engine, wasm_bytes: &[u8], export: &str) -> Result<()> {
    let module = wasmtime::Module::from_binary(engine, wasm_bytes)
        .map_err(|err| DeployerError::Other(format!("invalid provider runtime module: {err}")))?;
    let mut store = wasmtime::Store::new(engine, ());
    let instance = wasmtime::Instance::new(&mut store, &module, &[])
        .map_err(|err| DeployerError::Other(format!("failed to instantiate module: {err}")))?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| DeployerError::Other("runtime missing exported memory".into()))?;
    let func = instance
        .get_typed_func::<(), (i32, i32)>(&mut store, export)
        .map_err(|err| DeployerError::Other(format!("runtime export lookup failed: {err}")))?;
    let (ptr, len) = func
        .call(&mut store, ())
        .map_err(|err| DeployerError::Other(format!("validate-config call failed: {err}")))?;
    let data = memory
        .data(&store)
        .get(ptr as usize..ptr as usize + len as usize)
        .ok_or_else(|| DeployerError::Other("runtime returned invalid memory range".into()))?;
    interpret_validation_output(data)
}

fn interpret_validation_output(bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    let json: JsonValue =
        serde_json::from_slice(bytes).map_err(|err| DeployerError::Other(err.to_string()))?;
    let valid = json.get("valid").and_then(|v| v.as_bool()).ok_or_else(|| {
        DeployerError::Config("provider runtime result missing bool field 'valid'".into())
    })?;
    if valid {
        return Ok(());
    }
    let errors = json
        .get("errors")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let rendered = errors
        .iter()
        .map(|e| e.as_str().unwrap_or(&e.to_string()).to_string())
        .collect::<Vec<_>>()
        .join("; ");
    Err(DeployerError::Config(format!(
        "provider runtime rejected config: {}",
        if rendered.is_empty() {
            "invalid".to_string()
        } else {
            rendered
        }
    )))
}

fn persist_provider_config(
    greentic: &GreenticConfig,
    manifest: &PackManifest,
    provider: &ProviderDecl,
    instance_id: &str,
    config: &JsonValue,
    override_state: Option<&Path>,
    config_out: Option<&Path>,
) -> Result<PathBuf> {
    let path = if let Some(path) = config_out {
        PathBuf::from(path)
    } else {
        let state_dir = override_state
            .map(PathBuf::from)
            .unwrap_or_else(|| greentic.paths.state_dir.clone());
        state_dir
            .join("providers")
            .join(&provider.provider_type)
            .join(format!("{instance_id}.json"))
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let snapshot = StoredProviderConfig {
        provider_type: provider.provider_type.clone(),
        instance_id: instance_id.to_string(),
        pack_id: manifest.pack_id.to_string(),
        pack_version: manifest.version.to_string(),
        config_schema_ref: provider.config_schema_ref.clone(),
        runtime: RuntimeSnapshot {
            component_ref: provider.runtime.component_ref.clone(),
            export: provider.runtime.export.clone(),
            world: provider.runtime.world.clone(),
        },
        config: config.clone(),
    };
    let rendered = serde_json::to_string_pretty(&snapshot)
        .map_err(|err| DeployerError::Other(err.to_string()))?;
    fs::write(&path, rendered)?;
    Ok(path)
}

fn sanitize_instance_id(raw: String) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn verify_digest(expected: &str, bytes: &[u8]) -> Result<()> {
    let expected = expected.trim();
    if !expected.starts_with("sha256:") {
        return Err(DeployerError::Config(format!(
            "unsupported digest format {expected}; expected sha256:<hex>"
        )));
    }
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let actual = format!("sha256:{:x}", hasher.finalize());
    if expected != actual {
        return Err(DeployerError::Config(format!(
            "digest mismatch: expected {}, got {}",
            expected, actual
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpret_requires_valid_field() {
        let result = interpret_validation_output(br#"{"errors":["missing"]}"#);
        assert!(matches!(result, Err(DeployerError::Config(_))));
    }

    fn schema_with_secret() -> JsonValue {
        serde_json::json!({
            "type": "object",
            "required": ["token"],
            "properties": {
                "token": {"type": "string", "format": "password", "default": "secret"},
                "region": {"type": "string", "enum": ["us", "eu"], "title": "Region"},
                "notes": {"type": "string"}
            }
        })
    }

    #[test]
    fn questions_capture_defaults_and_secrets() {
        let schema = schema_with_secret();
        let questions = build_questions(&schema).expect("questions");
        assert_eq!(questions.len(), 3);
        let token = questions.iter().find(|q| q.id == "token").unwrap();
        assert!(token.secret);
        assert!(token.required);
        assert_eq!(token.default.as_ref().unwrap(), "secret");
        let region = questions.iter().find(|q| q.id == "region").unwrap();
        assert_eq!(region.title, "Region");
        assert!(region.options.as_ref().unwrap().iter().any(|v| v == "eu"));
    }

    #[test]
    fn coerce_enforces_enum_and_defaults() {
        let schema = schema_with_secret();
        let questions = build_questions(&schema).expect("questions");
        let region = questions.iter().find(|q| q.id == "region").unwrap();
        assert!(coerce_answer("us", region).is_ok());
        assert!(coerce_answer("apac", region).is_err());
        let token = questions.iter().find(|q| q.id == "token").unwrap();
        assert_eq!(
            coerce_answer("", token).unwrap(),
            JsonValue::String("secret".into())
        );
    }
}
