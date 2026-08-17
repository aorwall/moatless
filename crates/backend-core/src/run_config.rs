//! Run-configuration vocabulary shared by every sandbox runtime.
//!
//! A provision plan carries these types from the control band down to the
//! Kubernetes and Docker providers. They describe how a sandbox container is
//! sized and what its preview servers run, so they sit beside the sandbox port
//! policy rather than in any one runtime or in the workspace store.
//!
//! Every string field that is really a value — a CPU quantity, a memory
//! quantity, a storage medium, an environment-variable name, a port — is parsed
//! at deserialization. Parsing is strict: an unrecognised value fails the whole
//! decode rather than silently becoming a default. Serialization keeps the
//! original spelling, so persisted JSON round-trips byte for byte.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU16;
use std::str::FromStr;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

/// A quantity that failed the Kubernetes quantity grammar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} quantity '{value}': {reason}")]
pub struct QuantityParseError {
    kind: &'static str,
    value: String,
    reason: &'static str,
}

// =============================================================================
// Cpu
// =============================================================================

/// A Kubernetes CPU quantity — `"250m"`, `"2"`, `"500000u"`, `"1000000000n"`.
///
/// Keeps the original spelling for pass-through to the Kubernetes API and
/// exposes the parsed value in millicores for the Docker path and for metrics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cpu {
    raw: String,
    millicores: u64,
}

impl Cpu {
    /// The quantity exactly as it was written.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The quantity in millicores.
    pub fn millicores(&self) -> u64 {
        self.millicores
    }
}

impl FromStr for Cpu {
    type Err = QuantityParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = |reason: &'static str| QuantityParseError {
            kind: "CPU",
            value: value.to_string(),
            reason,
        };
        // Each suffix names how many of its units make one millicore.
        let (digits, scale) = split_suffix(value, &[("n", 1_000_000), ("u", 1_000), ("m", 1)]);
        // A suffixed form is an integer count of that unit. Without a suffix the
        // value is whole (possibly fractional) cores.
        let millicores = if let Some(per_millicore) = scale {
            let units: u64 = digits.parse().map_err(|_| invalid("not an integer"))?;
            units / per_millicore
        } else {
            let cores: f64 = digits.parse().map_err(|_| invalid("not a number"))?;
            if !cores.is_finite() || cores < 0.0 {
                return Err(invalid("must be a finite, non-negative number of cores"));
            }
            let millis = cores * 1000.0;
            if millis > u64::MAX as f64 {
                return Err(invalid("overflows u64 millicores"));
            }
            millis as u64
        };
        Ok(Self {
            raw: value.to_string(),
            millicores,
        })
    }
}

impl fmt::Display for Cpu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl Serialize for Cpu {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for Cpu {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(de::Error::custom)
    }
}

// =============================================================================
// Memory
// =============================================================================

/// A Kubernetes memory quantity — `"4Gi"`, `"512Mi"`, `"2k"`, `"4096"`.
///
/// Keeps the original spelling for pass-through to the Kubernetes API and
/// exposes the parsed value in bytes for the Docker path and for metrics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Memory {
    raw: String,
    bytes: u64,
}

impl Memory {
    /// The quantity exactly as it was written.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// The quantity in bytes.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl FromStr for Memory {
    type Err = QuantityParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let invalid = |reason: &'static str| QuantityParseError {
            kind: "memory",
            value: value.to_string(),
            reason,
        };
        let (digits, multiplier) = split_suffix(
            value,
            &[
                ("Ki", 1024),
                ("Mi", 1024 * 1024),
                ("Gi", 1024 * 1024 * 1024),
                ("Ti", 1024_u64.pow(4)),
                ("k", 1_000),
                ("M", 1_000_000),
                ("G", 1_000_000_000),
                ("T", 1_000_000_000_000),
            ],
        );
        let units: u64 = digits.parse().map_err(|_| invalid("not an integer"))?;
        let bytes = units
            .checked_mul(multiplier.unwrap_or(1))
            .ok_or_else(|| invalid("overflows u64 bytes"))?;
        Ok(Self {
            raw: value.to_string(),
            bytes,
        })
    }
}

impl fmt::Display for Memory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl Serialize for Memory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for Memory {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(de::Error::custom)
    }
}

/// Split the longest matching suffix off a quantity, returning the leading
/// digits and the suffix's scale. Suffixes are matched longest-first so `"Mi"`
/// wins over `"M"`.
fn split_suffix<'a>(value: &'a str, suffixes: &[(&str, u64)]) -> (&'a str, Option<u64>) {
    let mut best: Option<(&str, u64)> = None;
    for (suffix, scale) in suffixes {
        if value.ends_with(suffix) && best.is_none_or(|(seen, _)| suffix.len() > seen.len()) {
            best = Some((suffix, *scale));
        }
    }
    match best {
        Some((suffix, scale)) => (&value[..value.len() - suffix.len()], Some(scale)),
        None => (value, None),
    }
}

// =============================================================================
// StorageMedium
// =============================================================================

/// Where a node-local sandbox volume is backed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageMedium {
    /// A Kubernetes `emptyDir`, capped by the paired size quantity.
    EmptyDir,
    /// A node-local `hostPath` under `K8S_SANDBOX_CACHE_HOST_ROOT`. Kubernetes
    /// enforces no size cap; capacity is the node filesystem.
    Node,
}

impl StorageMedium {
    /// Every variant, so a conversion table cannot silently miss one.
    pub const ALL: &'static [Self] = &[Self::EmptyDir, Self::Node];

    /// The wire spelling — the same string serde emits.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EmptyDir => "emptyDir",
            Self::Node => "node",
        }
    }
}

impl fmt::Display for StorageMedium {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// =============================================================================
// EnvKey / EnvVars
// =============================================================================

/// An environment-variable name that satisfies the POSIX grammar:
/// `[A-Za-z_][A-Za-z0-9_]*`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct EnvKey(String);

impl EnvKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EnvKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for EnvKey {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut chars = value.chars();
        let valid = match chars.next() {
            Some(first) if first.is_ascii_alphabetic() || first == '_' => {
                chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            }
            _ => false,
        };
        if valid {
            Ok(Self(value.to_string()))
        } else {
            Err(format!(
                "Invalid environment variable name '{value}'. Must match [A-Za-z_][A-Za-z0-9_]*"
            ))
        }
    }
}

impl<'de> Deserialize<'de> for EnvKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(de::Error::custom)
    }
}

/// Environment overrides for one container, ordered by name.
///
/// The order is deterministic so a generated pod spec does not churn between
/// two provisions of the same config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EnvVars(BTreeMap<EnvKey, String>);

impl EnvVars {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&EnvKey, &String)> {
        self.0.iter()
    }
}

impl FromIterator<(EnvKey, String)> for EnvVars {
    fn from_iter<T: IntoIterator<Item = (EnvKey, String)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<'a> IntoIterator for &'a EnvVars {
    type Item = (&'a EnvKey, &'a String);
    type IntoIter = std::collections::btree_map::Iter<'a, EnvKey, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

// =============================================================================
// Port
// =============================================================================

/// A TCP port a preview server listens on. Zero is not a port, so an
/// unallocated server carries `None` rather than a sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Port(NonZeroU16);

impl Port {
    pub fn new(port: u16) -> Option<Self> {
        NonZeroU16::new(port).map(Self)
    }

    pub fn get(self) -> u16 {
        self.0.get()
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.get())
    }
}

// =============================================================================
// ResourceConfig
// =============================================================================

/// A container's CPU and memory for one direction (requests or limits).
///
/// Each field overrides the deployment default independently: a config that
/// names only `memory` keeps the configured CPU default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceQuantities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<Cpu>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<Memory>,
}

impl ResourceQuantities {
    pub fn is_empty(&self) -> bool {
        self.cpu.is_none() && self.memory.is_none()
    }
}

/// Resource and storage configuration for a sandbox, consumed by every runtime.
///
/// The same shape serves the workspace-wide block and a per-server override.
/// Only `requests`/`limits` are meaningful per server; the storage fields are
/// workspace-wide.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceConfig {
    #[serde(default, skip_serializing_if = "ResourceQuantities::is_empty")]
    pub requests: ResourceQuantities,
    #[serde(default, skip_serializing_if = "ResourceQuantities::is_empty")]
    pub limits: ResourceQuantities,
    /// Workspace storage size. Sizes the per-task PVC when `pvc_enabled`, and
    /// the node-local emptyDir otherwise. Unset falls back to
    /// `K8S_SANDBOX_STORAGE_SIZE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<Memory>,
    /// Whether this sandbox gets a per-task PVC mounted at `/opt/moatless`.
    /// Unset falls back to `K8S_SANDBOX_DEFAULT_PVC`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pvc_enabled: Option<bool>,
    /// Whether this sandbox snapshots its workspace to S3. Unset falls back to
    /// `K8S_SANDBOX_DEFAULT_S3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s3_enabled: Option<bool>,
    /// Size cap for the dind sidecar's `/var/lib/docker` emptyDir. Only
    /// meaningful with `docker_storage_mode: EmptyDir`. Unset falls back to
    /// `K8S_SANDBOX_DOCKER_DIND_STORAGE_SIZE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_storage: Option<Memory>,
    /// Where the dind sidecar's `/var/lib/docker` is backed. Unset means
    /// [`StorageMedium::Node`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_storage_mode: Option<StorageMedium>,
}

// =============================================================================
// ServerConfig
// =============================================================================

/// One preview server declared by a workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    /// `None` until a port is allocated — see `assign_missing_ports`.
    #[serde(default)]
    pub port: Option<Port>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub start_commands: Vec<String>,
    #[serde(default)]
    pub startup_timeout_seconds: Option<u32>,
    #[serde(default)]
    pub auto_reload: bool,
    #[serde(default)]
    pub enable_inspector: bool,
    #[serde(default)]
    pub default: bool,
    /// Per-server image. Falls back to the workspace image, then the sandbox
    /// image. An empty string reads as unset.
    #[serde(default, deserialize_with = "empty_string_as_none")]
    pub docker_image: Option<String>,
    #[serde(default, skip_serializing_if = "EnvVars::is_empty")]
    pub env_vars: EnvVars,
    /// Per-server requests/limits. Falls back to the workspace-wide block.
    #[serde(default)]
    pub resources: Option<ResourceConfig>,
}

fn empty_string_as_none<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(deserializer)?.filter(|s| !s.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T>(json: &str) -> T
    where
        T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let parsed: T = serde_json::from_str(json).expect("deserialize");
        assert_eq!(
            serde_json::to_string(&parsed).expect("serialize"),
            json,
            "serialization must reproduce the input spelling"
        );
        parsed
    }

    #[test]
    fn cpu_quantities_parse_to_millicores_and_keep_their_spelling() {
        for (json, millicores) in [
            (r#""250m""#, 250),
            (r#""1""#, 1000),
            (r#""2""#, 2000),
            (r#""0.5""#, 500),
            (r#""500000u""#, 500),
            (r#""1000000000n""#, 1000),
        ] {
            let cpu: Cpu = round_trip(json);
            assert_eq!(cpu.millicores(), millicores, "for {json}");
        }
    }

    #[test]
    fn a_malformed_cpu_quantity_fails_the_decode() {
        for json in [r#""garbage""#, r#""12x""#, r#""1.5m""#, r#""-1""#, r#""""#] {
            assert!(
                serde_json::from_str::<Cpu>(json).is_err(),
                "{json} must be rejected"
            );
        }
    }

    #[test]
    fn memory_quantities_parse_to_bytes_and_keep_their_spelling() {
        for (json, bytes) in [
            (r#""128Mi""#, 128 * 1024 * 1024),
            (r#""1Gi""#, 1024 * 1024 * 1024),
            (r#""256000Ki""#, 256_000 * 1024),
            (r#""2k""#, 2000),
            (r#""5M""#, 5_000_000),
            (r#""3G""#, 3_000_000_000),
            (r#""4096""#, 4096),
        ] {
            let memory: Memory = round_trip(json);
            assert_eq!(memory.bytes(), bytes, "for {json}");
        }
    }

    #[test]
    fn a_malformed_memory_quantity_fails_the_decode() {
        for json in [r#""garbage""#, r#""1.5Gi""#, r#""10Gb""#, r#""""#] {
            assert!(
                serde_json::from_str::<Memory>(json).is_err(),
                "{json} must be rejected"
            );
        }
    }

    #[test]
    fn storage_medium_maps_both_ways_and_rejects_anything_else() {
        for medium in StorageMedium::ALL {
            let json = serde_json::to_string(medium).expect("serialize");
            assert_eq!(json, format!("\"{}\"", medium.as_str()));
            assert_eq!(
                serde_json::from_str::<StorageMedium>(&json).expect("deserialize"),
                *medium
            );
            assert_eq!(medium.to_string(), medium.as_str());
        }
        for json in [r#""s3""#, r#""EMPTYDIR""#, r#""  node ""#, r#""emptydir""#] {
            assert!(
                serde_json::from_str::<StorageMedium>(json).is_err(),
                "{json} must be rejected"
            );
        }
    }

    #[test]
    fn env_keys_follow_the_posix_grammar() {
        for json in [r#""PATH""#, r#""_private""#, r#""a1_B2""#] {
            round_trip::<EnvKey>(json);
        }
        for json in [r#""1ABC""#, r#""WITH-DASH""#, r#""WITH SPACE""#, r#""""#] {
            assert!(
                serde_json::from_str::<EnvKey>(json).is_err(),
                "{json} must be rejected"
            );
        }
    }

    #[test]
    fn env_vars_serialize_in_name_order() {
        let vars: EnvVars = serde_json::from_str(r#"{"ZED":"3","ALPHA":"1","MID":"2"}"#)
            .expect("deserialize EnvVars");
        assert_eq!(
            serde_json::to_string(&vars).expect("serialize"),
            r#"{"ALPHA":"1","MID":"2","ZED":"3"}"#
        );
        assert_eq!(vars.len(), 3);
    }

    #[test]
    fn a_port_is_never_zero() {
        assert_eq!(round_trip::<Port>("3000").get(), 3000);
        assert!(serde_json::from_str::<Port>("0").is_err());
        assert!(serde_json::from_str::<Port>("70000").is_err());
        assert_eq!(Port::new(0), None);
        assert_eq!(Port::new(8080).map(Port::get), Some(8080));
    }

    #[test]
    fn resource_config_round_trips_without_changing_its_json() {
        let json = r#"{"requests":{"cpu":"500m","memory":"1Gi"},"limits":{"cpu":"4","memory":"8Gi"},"storage":"20Gi","pvcEnabled":true,"s3Enabled":false,"dockerStorage":"30Gi","dockerStorageMode":"node"}"#;
        let config: ResourceConfig = round_trip(json);
        assert_eq!(config.requests.cpu.as_ref().map(Cpu::millicores), Some(500));
        assert_eq!(
            config.limits.memory.as_ref().map(Memory::bytes),
            Some(8 * 1024 * 1024 * 1024)
        );
        assert_eq!(config.docker_storage_mode, Some(StorageMedium::Node));
    }

    #[test]
    fn an_empty_resource_config_emits_nothing() {
        assert_eq!(
            serde_json::to_string(&ResourceConfig::default()).expect("serialize"),
            "{}"
        );
    }

    #[test]
    fn a_stale_key_is_ignored_rather_than_failing_the_decode() {
        // Rows written before a field was removed keep their keys in JSONB, and
        // committed `.moatless/` config files keep theirs until someone edits
        // them. Neither may break a start.
        let config: ResourceConfig = serde_json::from_str(
            r#"{"storage":"20Gi","buildCacheMode":"node","buildCacheScope":"task"}"#,
        )
        .expect("deserialize");
        assert_eq!(config.storage.as_ref().map(Memory::as_str), Some("20Gi"));
    }

    #[test]
    fn one_bad_quantity_fails_the_whole_resource_config() {
        assert!(serde_json::from_str::<ResourceConfig>(r#"{"requests":{"cpu":"lots"}}"#).is_err());
        assert!(
            serde_json::from_str::<ResourceConfig>(r#"{"dockerStorageMode":"tmpfs"}"#).is_err()
        );
    }

    #[test]
    fn server_config_round_trips_without_changing_its_json() {
        let json = r#"{"name":"web","label":"Web","port":3000,"startCommands":["bun run dev"],"startupTimeoutSeconds":90,"autoReload":true,"enableInspector":true,"default":true,"dockerImage":"node:24","envVars":{"MODE":"dev"},"resources":{"limits":{"memory":"2Gi"}}}"#;
        let server: ServerConfig = round_trip(json);
        assert_eq!(server.port.map(Port::get), Some(3000));
        assert_eq!(server.start_commands, vec!["bun run dev".to_string()]);
        assert_eq!(server.env_vars.len(), 1);
    }

    #[test]
    fn a_minimal_server_config_uses_domain_defaults() {
        let server: ServerConfig =
            serde_json::from_str(r#"{"name":"files","port":4000}"#).expect("deserialize");
        assert_eq!(server.name, "files");
        assert_eq!(server.port.map(Port::get), Some(4000));
        assert!(!server.default);
        assert!(server.start_commands.is_empty());
        assert!(server.env_vars.is_empty());
        assert!(server.resources.is_none());
    }

    #[test]
    fn a_server_without_a_port_carries_none_rather_than_zero() {
        let server: ServerConfig =
            serde_json::from_str(r#"{"name":"files"}"#).expect("deserialize");
        assert_eq!(server.port, None);
        assert!(serde_json::from_str::<ServerConfig>(r#"{"name":"files","port":0}"#).is_err());
    }

    #[test]
    fn an_empty_docker_image_reads_as_unset() {
        let server: ServerConfig =
            serde_json::from_str(r#"{"name":"web","dockerImage":""}"#).expect("deserialize");
        assert_eq!(server.docker_image, None);
    }
}
