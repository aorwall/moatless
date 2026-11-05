//! The service proxies a deployment defines in `SERVICE_PROXIES_JSON`: each an
//! upstream a sandbox reaches through the backend, with a credential it never
//! holds and a deny-by-default rule list.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;

use crate::proxy_rules::{Rule, RuleSet, RuleSpec};

/// How the backend presents a service's credential upstream.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "style", rename_all = "camelCase", deny_unknown_fields)]
pub enum Injection {
    /// `Authorization: Bearer <secret>`.
    Bearer,
    /// `<name>: <prefix><secret>`.
    Header {
        name: String,
        #[serde(default)]
        prefix: String,
    },
    /// HTTP Basic with a fixed `username` and the secret as the password.
    Basic { username: String },
    /// `?<param>=<secret>`, replacing any value the caller sent.
    Query { param: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceCredential {
    /// The secret key, resolved per task like any other secret.
    pub secret: String,
    pub inject: Injection,
}

/// The env vars a sandbox gets for a service its Workspace has on.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Exposure {
    /// Set to the in-pod relay's URL for the service.
    pub url_env: String,
    /// Set to [`BROKERED_PLACEHOLDER`], for a tool that refuses to start
    /// without a token.
    #[serde(default)]
    pub token_env: Option<String>,
}

/// What a sandbox holds in place of a brokered secret.
pub const BROKERED_PLACEHOLDER: &str = "moatless-brokered";

/// Which Workspaces have a service on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Availability {
    /// Every Workspace, with no opt-in.
    All,
    /// Only a Workspace whose `WORKSPACE_IDENTITY_GRANTS` entry also lists it.
    #[default]
    Granted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Limits {
    #[serde(default = "default_request_body_bytes")]
    pub request_body_bytes: usize,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            request_body_bytes: default_request_body_bytes(),
            idle_timeout_secs: default_idle_timeout_secs(),
        }
    }
}

impl Limits {
    pub fn idle_timeout(&self) -> Duration {
        Duration::from_secs(self.idle_timeout_secs)
    }
}

fn default_request_body_bytes() -> usize {
    2 * 1024 * 1024
}

fn default_idle_timeout_secs() -> u64 {
    120
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DefinitionSpec {
    upstream: String,
    credential: ServiceCredential,
    #[serde(default)]
    expose: Option<Exposure>,
    #[serde(default)]
    availability: Availability,
    #[serde(default)]
    limits: Limits,
    rules: Vec<RuleSpec>,
}

/// One service, validated.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceProxy {
    pub name: String,
    /// `https://host[:port]`, with no path, query or userinfo.
    pub upstream: String,
    pub credential: ServiceCredential,
    pub expose: Option<Exposure>,
    pub availability: Availability,
    pub limits: Limits,
    pub rules: RuleSet,
}

/// Every service this deployment defines, by name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServiceProxies(BTreeMap<String, ServiceProxy>);

impl ServiceProxies {
    /// Parse the JSON object `SERVICE_PROXIES_JSON` holds. Blank is none.
    ///
    /// Any invalid definition fails the whole parse, so startup stops rather
    /// than serving a service with a rule list nobody reviewed.
    pub fn parse(json: Option<&str>) -> Result<Self, String> {
        let Some(json) = json.map(str::trim).filter(|j| !j.is_empty()) else {
            return Ok(Self::default());
        };
        let specs: BTreeMap<String, DefinitionSpec> =
            serde_json::from_str(json).map_err(|e| format!("SERVICE_PROXIES_JSON: {e}"))?;
        let mut services = BTreeMap::new();
        for (name, spec) in specs {
            let service =
                definition(&name, spec).map_err(|e| format!("SERVICE_PROXIES_JSON.{name}: {e}"))?;
            services.insert(name, service);
        }
        let parsed = Self(services);
        parsed.check_distinct_keys()?;
        Ok(parsed)
    }

    pub fn get(&self, name: &str) -> Option<&ServiceProxy> {
        self.0.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ServiceProxy> {
        self.0.values()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The service whose credential is `key`, if any defines it.
    pub fn brokering(&self, key: &str) -> Option<&ServiceProxy> {
        self.0.values().find(|s| s.credential.secret == key)
    }

    /// Refuses a definition whose secret or exposed env name `owned` claims,
    /// such as a reserved runtime key or a git or model credential.
    ///
    /// Separate from [`Self::parse`] because the key vocabularies live in
    /// crates above this one.
    pub fn check_keys(&self, owned: impl Fn(&str) -> bool) -> Result<(), String> {
        for service in self.0.values() {
            let at = |what: &str, key: &str| {
                format!(
                    "SERVICE_PROXIES_JSON.{}: {what} `{key}` is a key the platform owns",
                    service.name
                )
            };
            if owned(&service.credential.secret) {
                return Err(at("credential.secret", &service.credential.secret));
            }
            if let Some(expose) = &service.expose {
                if owned(&expose.url_env) {
                    return Err(at("expose.urlEnv", &expose.url_env));
                }
                if let Some(token_env) = expose.token_env.as_deref().filter(|k| owned(k)) {
                    return Err(at("expose.tokenEnv", token_env));
                }
            }
        }
        Ok(())
    }

    /// Two services exposing one env name would overwrite each other in the
    /// pod; one service's URL env naming another's secret would hand the
    /// relay URL to a tool expecting the secret.
    fn check_distinct_keys(&self) -> Result<(), String> {
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for service in self.0.values() {
            let mut names = vec![service.credential.secret.as_str()];
            if let Some(expose) = &service.expose {
                names.push(&expose.url_env);
                if let Some(token_env) = &expose.token_env {
                    // The token env usually reuses the secret's own name, so a
                    // tool reads the placeholder where it read the secret.
                    if token_env != &service.credential.secret {
                        names.push(token_env);
                    }
                }
            }
            for key in names {
                if let Some(other) = seen.insert(key, &service.name) {
                    return Err(format!(
                        "SERVICE_PROXIES_JSON: `{key}` is named by both {other} and {}",
                        service.name
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Collects already-built definitions without validating them again, for a
/// test that points a parsed definition at a local `http` upstream.
impl FromIterator<ServiceProxy> for ServiceProxies {
    fn from_iter<I: IntoIterator<Item = ServiceProxy>>(iter: I) -> Self {
        Self(iter.into_iter().map(|s| (s.name.clone(), s)).collect())
    }
}

fn definition(name: &str, spec: DefinitionSpec) -> Result<ServiceProxy, String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err("a service name is lowercase letters, digits and `-`".to_string());
    }
    let upstream = upstream_origin(&spec.upstream)?;
    if !is_env_name(&spec.credential.secret) {
        return Err(format!(
            "credential.secret `{}` is not an env var name",
            spec.credential.secret
        ));
    }
    match &spec.credential.inject {
        Injection::Header { name, .. } if name.is_empty() || name.contains(':') => {
            return Err("inject.name is not a header name".to_string());
        }
        Injection::Query { param } if param.is_empty() => {
            return Err("inject.param is empty".to_string());
        }
        _ => {}
    }
    if let Some(expose) = &spec.expose {
        for key in std::iter::once(&expose.url_env).chain(expose.token_env.as_ref()) {
            if !is_env_name(key) {
                return Err(format!(
                    "expose names `{key}`, which is not an env var name"
                ));
            }
        }
    }
    if spec.rules.is_empty() {
        return Err("rules is empty, so every request would be refused".to_string());
    }
    let rules = spec
        .rules
        .iter()
        .map(Rule::from_spec)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ServiceProxy {
        name: name.to_string(),
        upstream,
        credential: spec.credential,
        expose: spec.expose,
        availability: spec.availability,
        limits: spec.limits,
        rules: RuleSet::new(rules),
    })
}

/// `raw` as a bare `https` origin. A path, query, fragment or userinfo is
/// refused rather than dropped: each would let the definition say one host and
/// mean another.
fn upstream_origin(raw: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(raw).map_err(|e| format!("upstream `{raw}`: {e}"))?;
    if url.scheme() != "https" {
        return Err(format!("upstream `{raw}` is not https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(format!("upstream `{raw}` carries userinfo"));
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err(format!("upstream `{raw}` has a path, query or fragment"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| format!("upstream `{raw}` names no host"))?;
    Ok(match url.port() {
        Some(port) => format!("https://{host}:{port}"),
        None => format!("https://{host}"),
    })
}

fn is_env_name(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with(|c: char| c.is_ascii_digit())
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grafana() -> serde_json::Value {
        serde_json::json!({
            "upstream": "https://g-1.grafana-workspace.eu-central-1.amazonaws.com",
            "credential": { "secret": "GRAFANA_TOKEN", "inject": { "style": "bearer" } },
            "expose": { "urlEnv": "GRAFANA_URL", "tokenEnv": "GRAFANA_TOKEN" },
            "availability": "granted",
            "rules": [
                { "methods": ["GET"], "path": "/api/health" },
                { "methods": ["POST"], "path": "/api/ds/query" }
            ]
        })
    }

    fn parse_one(service: serde_json::Value) -> Result<ServiceProxies, String> {
        ServiceProxies::parse(Some(&serde_json::json!({ "grafana": service }).to_string()))
    }

    fn with(
        mut service: serde_json::Value,
        pointer: &str,
        value: serde_json::Value,
    ) -> serde_json::Value {
        *service.pointer_mut(pointer).unwrap() = value;
        service
    }

    #[test]
    fn a_valid_definition_parses_with_its_defaults() {
        let services = parse_one(grafana()).unwrap();
        let grafana = services.get("grafana").unwrap();
        assert_eq!(
            grafana.upstream,
            "https://g-1.grafana-workspace.eu-central-1.amazonaws.com"
        );
        assert_eq!(grafana.limits, Limits::default());
        assert_eq!(grafana.availability, Availability::Granted);
        assert_eq!(services.brokering("GRAFANA_TOKEN").unwrap().name, "grafana");
        assert!(services.brokering("GRAFANA_URL").is_none());
    }

    #[test]
    fn blank_config_defines_nothing() {
        assert!(ServiceProxies::parse(None).unwrap().is_empty());
        assert!(ServiceProxies::parse(Some("  ")).unwrap().is_empty());
    }

    #[test]
    fn an_upstream_that_could_mean_another_host_is_refused() {
        for upstream in [
            "http://grafana.example.com",
            "https://grafana.example.com/api",
            "https://user:pw@grafana.example.com",
            "https://grafana.example.com?x=1",
            "grafana.example.com",
        ] {
            let err = parse_one(with(grafana(), "/upstream", upstream.into())).unwrap_err();
            assert!(
                err.contains("SERVICE_PROXIES_JSON.grafana"),
                "{upstream}: {err}"
            );
        }
        let trailing = parse_one(with(
            grafana(),
            "/upstream",
            "https://grafana.example.com/".into(),
        ));
        assert_eq!(
            trailing.unwrap().get("grafana").unwrap().upstream,
            "https://grafana.example.com"
        );
    }

    #[test]
    fn a_bad_glob_an_empty_rule_list_or_an_unknown_field_fails_the_parse() {
        let bad_glob = with(grafana(), "/rules/0/path", "/api/*x".into());
        assert!(
            parse_one(bad_glob)
                .unwrap_err()
                .contains("must end its segment")
        );
        assert!(parse_one(with(grafana(), "/rules", serde_json::json!([]))).is_err());
        let mut typo = grafana();
        typo["rule"] = serde_json::json!([]);
        assert!(parse_one(typo).is_err());
    }

    #[test]
    fn a_secret_or_env_name_the_platform_owns_is_refused() {
        let services =
            parse_one(with(grafana(), "/credential/secret", "GITHUB_TOKEN".into())).unwrap();
        let err = services.check_keys(|k| k == "GITHUB_TOKEN").unwrap_err();
        assert!(err.contains("credential.secret `GITHUB_TOKEN`"), "{err}");

        let services = parse_one(with(grafana(), "/expose/urlEnv", "PATH".into())).unwrap();
        assert!(
            services
                .check_keys(|k| k == "PATH")
                .unwrap_err()
                .contains("expose.urlEnv")
        );
        assert!(parse_one(grafana()).unwrap().check_keys(|_| false).is_ok());
    }

    #[test]
    fn two_services_naming_one_key_are_refused() {
        let json = serde_json::json!({ "grafana": grafana(), "grafana-two": grafana() });
        let err = ServiceProxies::parse(Some(&json.to_string())).unwrap_err();
        assert!(err.contains("named by both"), "{err}");
    }
}
