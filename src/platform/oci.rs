use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::bootstrap::network::NetworkPolicy;
use crate::error::{DeployerError, Result};

const OCI_ACCEPT: &str = "application/vnd.oci.image.manifest.v1+json,application/vnd.docker.distribution.manifest.v2+json";
const PACK_MEDIA_TYPES: &[&str] = &[
    "application/vnd.greentic.pack.v1+gtpack",
    "application/octet-stream",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciReference {
    pub raw: String,
    pub host: String,
    pub repository: String,
    pub tag: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheIndex {
    entries: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct OciManifest {
    #[serde(default)]
    layers: Vec<OciLayer>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct OciLayer {
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
    size: Option<u64>,
}

pub fn parse_oci_reference(raw: &str) -> Result<OciReference> {
    if !raw.starts_with("oci://") {
        return Err(DeployerError::Pack(
            "oci reference must start with oci://".into(),
        ));
    }
    let rest = &raw["oci://".len()..];
    let (host, path) = rest
        .split_once('/')
        .ok_or_else(|| DeployerError::Pack("oci reference missing host/repo".into()))?;
    if host.is_empty() || path.is_empty() {
        return Err(DeployerError::Pack(
            "oci reference missing host or repository".into(),
        ));
    }
    let (repository, tag) = if let Some((repo, tag)) = path.rsplit_once(':') {
        (repo.to_string(), tag.to_string())
    } else {
        (path.to_string(), "latest".to_string())
    };
    Ok(OciReference {
        raw: raw.to_string(),
        host: host.to_string(),
        repository,
        tag,
    })
}

pub fn compute_sha256(path: &Path) -> Result<Option<String>> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let _ = std::io::copy(&mut file, &mut hasher)?;
    let bytes = hasher.finalize();
    Ok(Some(format!("sha256:{:x}", bytes)))
}

pub fn resolve_oci_pack(
    raw: &str,
    cache_base: &Path,
    network_policy: &NetworkPolicy,
) -> Result<PathBuf> {
    let reference = parse_oci_reference(raw)?;
    network_policy.enforce(&reference.host)?;
    let fetcher = HttpOciFetcher::new()?;
    resolve_with_fetcher_internal(reference, cache_base, &fetcher)
}

#[cfg(test)]
fn resolve_with_fetcher<F: OciFetcher>(
    raw: &str,
    cache_base: &Path,
    network_policy: &NetworkPolicy,
    fetcher: &F,
) -> Result<PathBuf> {
    let reference = parse_oci_reference(raw)?;
    network_policy.enforce(&reference.host)?;
    resolve_with_fetcher_internal(reference, cache_base, fetcher)
}

fn resolve_with_fetcher_internal<F: OciFetcher>(
    reference: OciReference,
    cache_base: &Path,
    fetcher: &F,
) -> Result<PathBuf> {
    let cache_dir = cache_base.join("cache");
    fs::create_dir_all(&cache_dir)?;
    let index_path = cache_dir.join("index.json");
    let mut index = load_index(&index_path)?;
    let key = cache_key(&reference);

    if let Some(digest) = index.entries.get(&key) {
        let cached = cache_dir.join(format!("{}.gtpack", digest.replace(':', "-")));
        if cached.exists() {
            return Ok(cached);
        }
    }

    let (manifest, manifest_digest, manifest_bytes) = fetcher.fetch_manifest(&reference)?;
    if let Some(expected) = manifest_digest.as_deref() {
        verify_digest(expected, &manifest_bytes, "manifest")?;
    }
    let layer = select_pack_layer(&manifest)
        .ok_or_else(|| DeployerError::Other("oci manifest does not contain any layers".into()))?;

    let bytes = fetcher.fetch_blob(&reference, &layer.digest)?;
    verify_digest(&layer.digest, &bytes, "pack blob")?;

    let digest = layer.digest.clone();
    let file_name = format!("{}.gtpack", digest.replace(':', "-"));
    let path = cache_dir.join(file_name);
    fs::write(&path, bytes)?;

    index.entries.insert(key, digest.clone());
    save_index(&index_path, &index)?;

    Ok(path)
}

trait OciFetcher {
    fn fetch_manifest(
        &self,
        reference: &OciReference,
    ) -> Result<(OciManifest, Option<String>, Vec<u8>)>;
    fn fetch_blob(&self, reference: &OciReference, digest: &str) -> Result<Vec<u8>>;
}

struct HttpOciFetcher {
    client: Client,
}

impl HttpOciFetcher {
    fn new() -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(Policy::none())
            .build()
            .map_err(|err| DeployerError::Other(err.to_string()))?;
        Ok(Self { client })
    }
}

impl OciFetcher for HttpOciFetcher {
    fn fetch_manifest(
        &self,
        reference: &OciReference,
    ) -> Result<(OciManifest, Option<String>, Vec<u8>)> {
        fetch_manifest_http(reference, &self.client)
    }

    fn fetch_blob(&self, reference: &OciReference, digest: &str) -> Result<Vec<u8>> {
        fetch_blob_http(reference, digest, &self.client)
    }
}

fn fetch_manifest_http(
    reference: &OciReference,
    client: &Client,
) -> Result<(OciManifest, Option<String>, Vec<u8>)> {
    let url = format!(
        "{}/v2/{}/manifests/{}",
        registry_base(reference),
        reference.repository,
        reference.tag
    );
    let response = client
        .get(&url)
        .header(reqwest::header::ACCEPT, OCI_ACCEPT)
        .send()
        .map_err(|err| {
            DeployerError::Other(format!("failed to fetch OCI manifest from {url}: {err}"))
        })?;
    if !response.status().is_success() {
        return Err(DeployerError::Other(format!(
            "fetch {url} failed with status {}",
            response.status()
        )));
    }
    let digest_header = response
        .headers()
        .get("docker-content-digest")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = response
        .bytes()
        .map_err(|err| DeployerError::Other(format!("failed to read manifest body: {err}")))?
        .to_vec();
    let manifest: OciManifest = serde_json::from_slice(&bytes)
        .map_err(|err| DeployerError::Other(format!("invalid OCI manifest from {url}: {err}")))?;
    Ok((manifest, digest_header, bytes))
}

fn select_pack_layer(manifest: &OciManifest) -> Option<&OciLayer> {
    manifest
        .layers
        .iter()
        .find(|layer| PACK_MEDIA_TYPES.contains(&layer.media_type.as_str()))
        .or_else(|| manifest.layers.first())
}

fn fetch_blob_http(reference: &OciReference, digest: &str, client: &Client) -> Result<Vec<u8>> {
    let url = format!(
        "{}/v2/{}/blobs/{}",
        registry_base(reference),
        reference.repository,
        digest
    );
    let response = client.get(&url).send().map_err(|err| {
        DeployerError::Other(format!("failed to fetch OCI blob from {url}: {err}"))
    })?;
    if !response.status().is_success() {
        return Err(DeployerError::Other(format!(
            "fetch {url} failed with status {}",
            response.status()
        )));
    }
    response
        .bytes()
        .map(|b| b.to_vec())
        .map_err(|err| DeployerError::Other(format!("failed to read blob body: {err}")))
}

fn registry_base(reference: &OciReference) -> String {
    if reference.host.contains("localhost")
        || reference.host.starts_with("127.")
        || reference.host.starts_with("[::1]")
    {
        format!("http://{}", reference.host)
    } else {
        format!("https://{}", reference.host)
    }
}

fn verify_digest(expected: &str, bytes: &[u8], context: &str) -> Result<()> {
    let computed = sha256_bytes(bytes);
    if expected != computed {
        return Err(DeployerError::Other(format!(
            "{} digest mismatch: expected {}, got {}",
            context, expected, computed
        )));
    }
    Ok(())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn cache_key(reference: &OciReference) -> String {
    format!(
        "{}/{}:{}",
        reference.host, reference.repository, reference.tag
    )
}

fn load_index(path: &Path) -> Result<CacheIndex> {
    if !path.exists() {
        return Ok(CacheIndex::default());
    }
    let mut buf = String::new();
    fs::File::open(path)?.read_to_string(&mut buf)?;
    serde_json::from_str(&buf)
        .map_err(|err| DeployerError::Other(format!("cache index parse error: {err}")))
}

fn save_index(path: &Path, index: &CacheIndex) -> Result<()> {
    let content = serde_json::to_string_pretty(index)
        .map_err(|err| DeployerError::Other(format!("cache index encode error: {err}")))?;
    fs::write(path, content)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use greentic_types::ComponentId;
    use greentic_types::PackId;
    use greentic_types::component::{ComponentCapabilities, ComponentManifest, ComponentProfiles};
    use greentic_types::flow::FlowKind;
    use greentic_types::pack_manifest::{PackKind, PackManifest};
    use semver::Version;
    use tar::Builder;
    use tempfile::tempdir;

    fn sample_gtpack_bytes() -> Vec<u8> {
        let manifest = PackManifest {
            schema_version: "pack-v1".to_string(),
            pack_id: PackId::try_from("dev.greentic.platform").unwrap(),
            version: Version::new(0, 1, 0),
            kind: PackKind::Application,
            publisher: "greentic".to_string(),
            components: vec![ComponentManifest {
                id: ComponentId::try_from("dev.greentic.platform.installer").unwrap(),
                version: Version::new(0, 1, 0),
                supports: vec![FlowKind::Messaging],
                world: "greentic:test/world".to_string(),
                profiles: ComponentProfiles::default(),
                capabilities: ComponentCapabilities::default(),
                configurators: None,
                operations: Vec::new(),
                config_schema: None,
                resources: Default::default(),
                dev_flows: Default::default(),
            }],
            flows: Vec::new(),
            dependencies: Vec::new(),
            capabilities: Vec::new(),
            secret_requirements: Vec::new(),
            signatures: Default::default(),
            bootstrap: None,
            extensions: None,
        };
        let encoded = greentic_types::cbor::encode_pack_manifest(&manifest).unwrap();
        let mut builder = Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_size(encoded.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "manifest.cbor", encoded.as_slice())
            .unwrap();
        builder.into_inner().unwrap()
    }

    #[test]
    fn parse_rejects_invalid_refs() {
        assert!(parse_oci_reference("gtpack").is_err());
        assert!(parse_oci_reference("oci://").is_err());
        assert!(parse_oci_reference("oci://registry").is_err());
    }

    #[test]
    fn parse_accepts_valid_refs() {
        let r = parse_oci_reference("oci://r.local/repo:1.0").unwrap();
        assert_eq!(r.host, "r.local");
        assert_eq!(r.repository, "repo");
        assert_eq!(r.tag, "1.0");
    }

    #[test]
    fn resolve_requires_policy_and_allowlist() {
        let dir = tempdir().unwrap();
        let offline = NetworkPolicy::new(
            false,
            true,
            crate::bootstrap::network::NetAllowList::default(),
        );
        let err = resolve_oci_pack("oci://r.local/repo:1.0", dir.path(), &offline).unwrap_err();
        assert!(err.to_string().contains("offline-only"));

        let disallowed = NetworkPolicy::new(
            false,
            false,
            crate::bootstrap::network::NetAllowList::default(),
        );
        let err = resolve_oci_pack("oci://r.local/repo:1.0", dir.path(), &disallowed).unwrap_err();
        assert!(err.to_string().contains("network access disabled"));

        let missing_allowlist = NetworkPolicy::new(
            true,
            false,
            crate::bootstrap::network::NetAllowList::default(),
        );
        let err =
            resolve_oci_pack("oci://r.local/repo:1.0", dir.path(), &missing_allowlist).unwrap_err();
        assert!(err.to_string().contains("allowlist"));
    }

    #[test]
    fn resolves_and_caches_with_mock_server() {
        let dir = tempdir().unwrap();
        let bytes = sample_gtpack_bytes();
        let pack_digest = sha256_bytes(&bytes);
        let fetcher = StubFetcher::new(bytes.clone(), None, None);
        let allowlist =
            crate::bootstrap::network::NetAllowList::parse(Some("registry.local")).unwrap();
        let policy = NetworkPolicy::new(true, false, allowlist);
        let url = "oci://registry.local/repo:latest";

        let path = resolve_with_fetcher(url, dir.path(), &policy, &fetcher).unwrap();
        assert!(path.exists());
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            format!("{}.gtpack", pack_digest.replace(':', "-"))
        );

        // Cached lookup should not hit the network.
        let cached = resolve_with_fetcher(url, dir.path(), &policy, &fetcher).unwrap();
        assert_eq!(cached, path);
    }

    #[test]
    fn fails_when_blob_digest_mismatches() {
        let dir = tempdir().unwrap();
        let bytes = sample_gtpack_bytes();
        let bad_digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        let fetcher = StubFetcher::new(bytes.clone(), Some(bad_digest.to_string()), None);
        let allowlist =
            crate::bootstrap::network::NetAllowList::parse(Some("registry.local")).unwrap();
        let policy = NetworkPolicy::new(true, false, allowlist);
        let url = "oci://registry.local/repo:latest";

        let err = resolve_with_fetcher(url, dir.path(), &policy, &fetcher).unwrap_err();
        assert!(err.to_string().contains("digest mismatch"));
    }

    struct StubFetcher {
        manifest: OciManifest,
        manifest_digest: Option<String>,
        manifest_bytes: Vec<u8>,
        blob: Vec<u8>,
    }

    impl StubFetcher {
        fn new(
            blob: Vec<u8>,
            digest_override: Option<String>,
            manifest_digest: Option<String>,
        ) -> Self {
            let digest = digest_override.unwrap_or_else(|| sha256_bytes(&blob));
            let layer = OciLayer {
                media_type: PACK_MEDIA_TYPES[0].to_string(),
                digest: digest.clone(),
                size: Some(blob.len() as u64),
            };
            let manifest = OciManifest {
                layers: vec![layer],
            };
            let manifest_bytes = serde_json::to_vec(&manifest).expect("serialize manifest");
            let manifest_digest = manifest_digest.or_else(|| Some(sha256_bytes(&manifest_bytes)));
            Self {
                manifest,
                manifest_digest,
                manifest_bytes,
                blob,
            }
        }
    }

    impl OciFetcher for StubFetcher {
        fn fetch_manifest(
            &self,
            _reference: &OciReference,
        ) -> Result<(OciManifest, Option<String>, Vec<u8>)> {
            Ok((
                self.manifest.clone(),
                self.manifest_digest.clone(),
                self.manifest_bytes.clone(),
            ))
        }

        fn fetch_blob(&self, _reference: &OciReference, _digest: &str) -> Result<Vec<u8>> {
            Ok(self.blob.clone())
        }
    }
}
