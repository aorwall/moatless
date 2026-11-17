//! The deny-by-default rule list a sandbox-facing proxy matches a request
//! against: a method set, a path glob and an optional rule on the JSON body.

use serde::Deserialize;

/// Whether a proxy acts on a request outside its policy or only logs it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProxyPolicyMode {
    #[default]
    Log,
    Enforce,
}

impl std::str::FromStr for ProxyPolicyMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "log" => Ok(Self::Log),
            "enforce" => Ok(Self::Enforce),
            other => Err(format!("`{other}` is neither `log` nor `enforce`")),
        }
    }
}

/// A request path made safe to match: percent-decoded once, with no `.` or
/// `..` segment, no empty segment, no backslash and no control byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalPath(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("the path is not absolute")]
    NotAbsolute,
    #[error("the path holds a malformed or double percent-encoding")]
    Encoding,
    #[error("the path holds a `.` or `..` segment, an empty segment or a backslash")]
    Traversal,
}

impl NormalPath {
    /// Normalise the path half of `path_and_query`; the query is ignored.
    ///
    /// Refused rather than cleaned: a path the upstream would resolve
    /// differently from this matcher is the trick the rules exist to stop.
    pub fn parse(path_and_query: &str) -> Result<Self, PathError> {
        let raw = path_and_query.split('?').next().unwrap_or_default();
        if !raw.starts_with('/') {
            return Err(PathError::NotAbsolute);
        }
        let decoded = percent_decode(raw)?;
        if decoded.contains('%') {
            return Err(PathError::Encoding);
        }
        if decoded.contains('\\') || decoded.chars().any(char::is_control) {
            return Err(PathError::Traversal);
        }
        // An empty last segment is a trailing `/`, which upstreams treat as the
        // same resource; anywhere else it is a `//`.
        let decoded_rest = &decoded[1..];
        let segments: Vec<&str> = decoded_rest.split('/').collect();
        let last = segments.len().saturating_sub(1);
        for (i, segment) in segments.iter().enumerate() {
            if matches!(*segment, "." | "..") || (segment.is_empty() && i != last) {
                return Err(PathError::Traversal);
            }
        }
        Ok(Self(decoded))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn segments(&self) -> Vec<&str> {
        let rest = self.0.trim_start_matches('/');
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.is_empty() {
            Vec::new()
        } else {
            rest.split('/').collect()
        }
    }
}

fn percent_decode(raw: &str) -> Result<String, PathError> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3).ok_or(PathError::Encoding)?;
            let hex = std::str::from_utf8(hex).map_err(|_| PathError::Encoding)?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| PathError::Encoding)?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| PathError::Encoding)
}

/// A path glob: `*` matches one segment, or the rest of one when it ends a
/// pattern segment (`query*`); `**` matches any number of segments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathGlob {
    source: String,
    segments: Vec<GlobSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GlobSegment {
    Any,
    Recursive,
    Literal(String),
    Prefix(String),
}

impl PathGlob {
    /// Parse `source`, which starts with `/`. A `*` anywhere but alone or at
    /// the end of a segment, or a `**` sharing its segment, is refused.
    pub fn parse(source: &str) -> Result<Self, String> {
        let rest = source
            .strip_prefix('/')
            .ok_or_else(|| format!("path glob `{source}` must start with `/`"))?;
        let mut segments = Vec::new();
        if !rest.is_empty() {
            for segment in rest.split('/') {
                segments.push(match segment {
                    "" => return Err(format!("path glob `{source}` has an empty segment")),
                    "**" => GlobSegment::Recursive,
                    "*" => GlobSegment::Any,
                    s if s.contains("**") => {
                        return Err(format!(
                            "path glob `{source}`: `**` must be a whole segment"
                        ));
                    }
                    s => match s.find('*') {
                        None => GlobSegment::Literal(s.to_string()),
                        Some(at) if at == s.len() - 1 => GlobSegment::Prefix(s[..at].to_string()),
                        Some(_) => {
                            return Err(format!("path glob `{source}`: `*` must end its segment"));
                        }
                    },
                });
            }
        }
        Ok(Self {
            source: source.to_string(),
            segments,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.source
    }

    pub fn matches(&self, path: &NormalPath) -> bool {
        glob_match(&self.segments, &path.segments())
    }
}

fn glob_match(pattern: &[GlobSegment], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((GlobSegment::Recursive, rest)) => {
            (0..=path.len()).any(|skip| glob_match(rest, &path[skip..]))
        }
        Some((head, rest)) => match path.split_first() {
            None => false,
            Some((segment, path_rest)) => {
                let hit = match head {
                    GlobSegment::Any => true,
                    GlobSegment::Literal(literal) => literal == segment,
                    GlobSegment::Prefix(prefix) => segment.starts_with(prefix.as_str()),
                    GlobSegment::Recursive => false,
                };
                hit && glob_match(rest, path_rest)
            }
        },
    }
}

/// A rule on the request's JSON body: the value at `pointer` must be one of
/// `one_of`. A missing value matches only when `one_of` holds `null`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BodyRule {
    /// An RFC 6901 JSON pointer, such as `/event`.
    pub pointer: String,
    pub one_of: Vec<serde_json::Value>,
}

impl BodyRule {
    /// False for a body that is not JSON.
    pub fn matches(&self, body: &[u8]) -> bool {
        let Ok(json) = serde_json::from_slice::<serde_json::Value>(body) else {
            return false;
        };
        let value = json
            .pointer(&self.pointer)
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        self.one_of.contains(&value)
    }
}

/// One allow rule as configuration writes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleSpec {
    pub methods: Vec<String>,
    pub path: String,
    #[serde(default)]
    pub body: Option<BodyRule>,
    /// Limits the rule to sandboxes whose pod runs as one of these Kubernetes
    /// service accounts. Absent means every caller.
    #[serde(default)]
    pub service_accounts: Option<Vec<String>>,
}

/// One allow rule, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    methods: Vec<String>,
    path: PathGlob,
    body: Option<BodyRule>,
    service_accounts: Option<Vec<String>>,
}

impl Rule {
    /// `methods` are upper-cased; an empty list or a glob that does not parse
    /// is refused.
    pub fn new(methods: &[&str], path: &str, body: Option<BodyRule>) -> Result<Self, String> {
        if methods.is_empty() {
            return Err(format!("rule `{path}` names no method"));
        }
        Ok(Self {
            methods: methods.iter().map(|m| m.to_ascii_uppercase()).collect(),
            path: PathGlob::parse(path)?,
            body,
            service_accounts: None,
        })
    }

    /// Limit the rule to callers running as one of `accounts`. An empty or
    /// blank list is refused: it would read as "everyone" to a reviewer and
    /// match no one.
    pub fn for_service_accounts(mut self, accounts: Vec<String>) -> Result<Self, String> {
        if accounts.is_empty() || accounts.iter().any(|a| a.trim().is_empty()) {
            return Err(format!(
                "rule `{}` has an empty serviceAccounts entry or list",
                self.path()
            ));
        }
        self.service_accounts = Some(accounts);
        Ok(self)
    }

    pub fn from_spec(spec: &RuleSpec) -> Result<Self, String> {
        let methods: Vec<&str> = spec.methods.iter().map(String::as_str).collect();
        let rule = Self::new(&methods, &spec.path, spec.body.clone())?;
        match &spec.service_accounts {
            Some(accounts) => rule.for_service_accounts(accounts.clone()),
            None => Ok(rule),
        }
    }

    pub fn path(&self) -> &str {
        self.path.as_str()
    }

    pub fn needs_body(&self) -> bool {
        self.body.is_some()
    }

    /// Whether a caller whose pod runs as `account` may use the rule. A caller
    /// with no recorded account never passes a limited rule.
    fn admits(&self, account: Option<&str>) -> bool {
        self.service_accounts
            .as_ref()
            .is_none_or(|accounts| account.is_some_and(|a| accounts.iter().any(|x| x == a)))
    }
}

/// An ordered allow list: the first rule whose method and path match and whose
/// service accounts admit the caller decides, and no match denies.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleSet(Vec<Rule>);

/// Why a [`RuleSet`] answered as it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Allowed by the rule with this path glob.
    Allow(String),
    /// A rule matched the method and path but its body rule did not.
    BodyRefused(String),
    /// No rule matched the method and path.
    NoRule,
}

impl RuleSet {
    pub fn new(rules: Vec<Rule>) -> Self {
        Self(rules)
    }

    /// Whether a request for `method` and `path` needs its body read before
    /// [`Self::check`] can answer.
    pub fn needs_body(&self, method: &str, path: &NormalPath, account: Option<&str>) -> bool {
        self.first(method, path, account)
            .is_some_and(Rule::needs_body)
    }

    /// The verdict for one request from a pod running as `account`. `body` is
    /// `None` when it was streamed unread, which a rule with a body rule
    /// refuses.
    pub fn check(
        &self,
        method: &str,
        path: &NormalPath,
        account: Option<&str>,
        body: Option<&[u8]>,
    ) -> Verdict {
        match self.first(method, path, account) {
            None => Verdict::NoRule,
            Some(rule) => match &rule.body {
                Some(body_rule) if !body.is_some_and(|b| body_rule.matches(b)) => {
                    Verdict::BodyRefused(rule.path().to_string())
                }
                _ => Verdict::Allow(rule.path().to_string()),
            },
        }
    }

    fn first(&self, method: &str, path: &NormalPath, account: Option<&str>) -> Option<&Rule> {
        self.0.iter().find(|rule| {
            rule.methods.iter().any(|m| m.eq_ignore_ascii_case(method))
                && rule.path.matches(path)
                && rule.admits(account)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(p: &str) -> NormalPath {
        NormalPath::parse(p).unwrap()
    }

    fn glob(p: &str) -> PathGlob {
        PathGlob::parse(p).unwrap()
    }

    #[test]
    fn a_single_star_matches_one_segment_and_a_double_star_any_number() {
        assert!(glob("/api/dashboards/uid/*").matches(&path("/api/dashboards/uid/abc")));
        assert!(!glob("/api/dashboards/uid/*").matches(&path("/api/dashboards/uid/abc/x")));
        assert!(!glob("/api/dashboards/uid/*").matches(&path("/api/dashboards/uid")));
        assert!(glob("/api/datasources/**").matches(&path("/api/datasources")));
        assert!(glob("/api/datasources/**").matches(&path("/api/datasources/uid/x/health")));
        assert!(glob("/a/**/c").matches(&path("/a/b/b/c")));
        assert!(glob("/**").matches(&path("/")));
    }

    #[test]
    fn a_trailing_star_matches_the_rest_of_its_segment() {
        let rule = glob("/api/datasources/proxy/uid/*/loki/api/v1/query*");
        assert!(rule.matches(&path(
            "/api/datasources/proxy/uid/l1/loki/api/v1/query_range"
        )));
        assert!(rule.matches(&path("/api/datasources/proxy/uid/l1/loki/api/v1/query?q=1")));
        assert!(!rule.matches(&path("/api/datasources/proxy/uid/l1/loki/api/v1/labels")));
    }

    #[test]
    fn a_malformed_glob_is_refused() {
        for bad in ["api/x", "/a//b", "/a/b**", "/a/*b", "/a/**x"] {
            assert!(PathGlob::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_encoded_traversal_is_refused_before_matching() {
        for bad in [
            "/api/%2e%2e/admin",
            "/api/../admin",
            "/api/./x",
            "/api//x",
            "/api/x%5cy",
            "/api/x\\y",
            "/api/%252e%252e/x",
            "/api/%zz",
            "/api/%2",
            "api/x",
        ] {
            assert!(NormalPath::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_encoded_slash_becomes_a_segment_boundary() {
        let p = path("/repos/o/r/branches/feat%2Fx");
        assert_eq!(p.as_str(), "/repos/o/r/branches/feat/x");
        assert!(glob("/repos/*/*/branches/**").matches(&p));
        assert!(!glob("/repos/*/*/branches/*").matches(&p));
    }

    #[test]
    fn a_trailing_slash_and_the_query_do_not_change_the_match() {
        assert!(glob("/api/search").matches(&path("/api/search/")));
        assert!(glob("/api/search").matches(&path("/api/search?query=x")));
    }

    fn rules() -> RuleSet {
        RuleSet::new(vec![
            Rule::new(
                &["post"],
                "/repos/*/*/pulls/*/reviews",
                Some(BodyRule {
                    pointer: "/event".to_string(),
                    one_of: vec![
                        serde_json::json!("COMMENT"),
                        serde_json::json!("REQUEST_CHANGES"),
                        serde_json::Value::Null,
                    ],
                }),
            )
            .unwrap(),
            Rule::new(&["GET"], "/repos/**", None).unwrap(),
        ])
    }

    #[test]
    fn the_first_rule_matching_method_and_path_decides() {
        let rules = rules();
        assert_eq!(
            rules.check("GET", &path("/repos/o/r/pulls/1"), None, None),
            Verdict::Allow("/repos/**".to_string())
        );
        assert_eq!(
            rules.check("DELETE", &path("/repos/o/r"), None, None),
            Verdict::NoRule
        );
        assert_eq!(
            rules.check("GET", &path("/user"), None, None),
            Verdict::NoRule
        );
    }

    #[test]
    fn a_body_rule_reads_the_json_pointer() {
        let rules = rules();
        let reviews = path("/repos/o/r/pulls/1/reviews");
        assert!(rules.needs_body("POST", &reviews, None));
        assert!(!rules.needs_body("GET", &reviews, None));
        let check = |body: &str| rules.check("POST", &reviews, None, Some(body.as_bytes()));
        assert!(matches!(check(r#"{"event":"COMMENT"}"#), Verdict::Allow(_)));
        assert!(matches!(check(r#"{"body":"pending"}"#), Verdict::Allow(_)));
        assert!(matches!(
            check(r#"{"event":"APPROVE"}"#),
            Verdict::BodyRefused(_)
        ));
        assert!(matches!(check("not json"), Verdict::BodyRefused(_)));
        assert!(matches!(
            rules.check("POST", &reviews, None, None),
            Verdict::BodyRefused(_)
        ));
    }

    #[test]
    fn a_limited_rule_admits_only_its_service_accounts() {
        let rules = RuleSet::new(vec![
            Rule::new(&["GET"], "/api/datasources/proxy/uid/*/**", None)
                .unwrap()
                .for_service_accounts(vec!["moatless-ops".to_string()])
                .unwrap(),
            Rule::new(&["GET"], "/api/datasources/proxy/uid/loki/**", None).unwrap(),
        ]);
        let prod = path("/api/datasources/proxy/uid/loki-prod/loki/api/v1/labels");
        let nonprod = path("/api/datasources/proxy/uid/loki/loki/api/v1/labels");
        let check = |p: &NormalPath, account| rules.check("GET", p, account, None);
        assert!(matches!(
            check(&prod, Some("moatless-ops")),
            Verdict::Allow(_)
        ));
        assert_eq!(check(&prod, Some("moatless-user")), Verdict::NoRule);
        assert_eq!(check(&prod, None), Verdict::NoRule);
        // A caller the limited rule skips falls through to the next rule.
        assert_eq!(
            check(&nonprod, None),
            Verdict::Allow("/api/datasources/proxy/uid/loki/**".to_string())
        );
    }

    #[test]
    fn an_empty_service_account_list_is_refused() {
        let rule = || Rule::new(&["GET"], "/x", None).unwrap();
        assert!(rule().for_service_accounts(vec![]).is_err());
        assert!(rule().for_service_accounts(vec![" ".to_string()]).is_err());
        let spec: RuleSpec = serde_json::from_value(serde_json::json!({
            "methods": ["GET"], "path": "/x", "serviceAccounts": []
        }))
        .unwrap();
        assert!(Rule::from_spec(&spec).is_err());
    }
}
