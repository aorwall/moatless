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
/// `..` segment, no empty segment, no backslash and no control byte. The raw
/// query rides along for a [`QueryRule`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalPath {
    path: String,
    query: String,
}

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
    /// Normalise the path half of `path_and_query`, keeping the query raw.
    ///
    /// Refused rather than cleaned: a path the upstream would resolve
    /// differently from this matcher is the trick the rules exist to stop.
    pub fn parse(path_and_query: &str) -> Result<Self, PathError> {
        let (raw, query) = path_and_query
            .split_once('?')
            .unwrap_or((path_and_query, ""));
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
        Ok(Self {
            path: decoded,
            query: query.to_string(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.path
    }

    /// The raw query after `?`, empty when there is none.
    pub fn query(&self) -> &str {
        &self.query
    }

    fn segments(&self) -> Vec<&str> {
        let rest = self.path.trim_start_matches('/');
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
            // `from_str_radix` alone takes `+f`, which `net/url` refuses.
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return Err(PathError::Encoding);
            }
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

/// A rule on the request's query string: `param` must appear exactly once,
/// and its form-decoded value must be one of `one_of`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryRule {
    pub param: String,
    pub one_of: Vec<String>,
}

impl QueryRule {
    /// False for a repeated `param`, since upstreams disagree on which copy
    /// they read, and for any pair that does not decode.
    pub fn matches(&self, query: &str) -> bool {
        let mut found = None;
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let (Some(key), Some(value)) = (form_decode(key), form_decode(value)) else {
                return false;
            };
            if key == self.param {
                if found.is_some() {
                    return false;
                }
                found = Some(value);
            }
        }
        found.is_some_and(|value| self.one_of.contains(&value))
    }
}

/// `application/x-www-form-urlencoded`: `+` is a space, then percent-decoding.
fn form_decode(raw: &str) -> Option<String> {
    percent_decode(&raw.replace('+', " ")).ok()
}

/// One allow rule as configuration writes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuleSpec {
    pub methods: Vec<String>,
    pub path: String,
    #[serde(default)]
    pub body: Option<BodyRule>,
    #[serde(default)]
    pub query: Option<QueryRule>,
    /// Limits the rule to sandboxes whose pod runs as one of these Kubernetes
    /// service accounts. Absent means every caller.
    #[serde(default)]
    pub service_accounts: Option<Vec<String>>,
    /// Limits the rule to sandboxes acting for one of these Moatless user ids.
    /// With `service_accounts` too, a caller matching either list passes.
    #[serde(default)]
    pub users: Option<Vec<String>>,
}

/// Who a proxied request comes from, as a limited [`Rule`] reads it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Caller<'a> {
    /// The pod's recorded service account; `None` for the default.
    pub service_account: Option<&'a str>,
    /// The Moatless user id the sandbox acts for.
    pub user: Option<&'a str>,
}

/// One allow rule, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    methods: Vec<String>,
    path: PathGlob,
    body: Option<BodyRule>,
    query: Option<QueryRule>,
    service_accounts: Option<Vec<String>>,
    users: Option<Vec<String>>,
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
            query: None,
            service_accounts: None,
            users: None,
        })
    }

    /// Limit the rule to callers running as one of `accounts`. An empty or
    /// blank list is refused: it would read as "everyone" to a reviewer and
    /// match no one.
    pub fn for_service_accounts(mut self, accounts: Vec<String>) -> Result<Self, String> {
        self.service_accounts = Some(non_empty(accounts, "serviceAccounts", self.path())?);
        Ok(self)
    }

    /// Require the request's query to satisfy `query`. An empty `one_of` is
    /// refused: it would match no request.
    pub fn with_query(mut self, query: QueryRule) -> Result<Self, String> {
        if query.param.trim().is_empty() || query.one_of.is_empty() {
            return Err(format!(
                "rule `{}` has a query rule with no param or no values",
                self.path()
            ));
        }
        self.query = Some(query);
        Ok(self)
    }

    /// Limit the rule to callers acting for one of `users`, refused when empty
    /// as [`Self::for_service_accounts`] is.
    pub fn for_users(mut self, users: Vec<String>) -> Result<Self, String> {
        self.users = Some(non_empty(users, "users", self.path())?);
        Ok(self)
    }

    pub fn from_spec(spec: &RuleSpec) -> Result<Self, String> {
        let methods: Vec<&str> = spec.methods.iter().map(String::as_str).collect();
        let mut rule = Self::new(&methods, &spec.path, spec.body.clone())?;
        if let Some(accounts) = &spec.service_accounts {
            rule = rule.for_service_accounts(accounts.clone())?;
        }
        if let Some(users) = &spec.users {
            rule = rule.for_users(users.clone())?;
        }
        if let Some(query) = &spec.query {
            rule = rule.with_query(query.clone())?;
        }
        Ok(rule)
    }

    pub fn path(&self) -> &str {
        self.path.as_str()
    }

    pub fn needs_body(&self) -> bool {
        self.body.is_some()
    }

    /// Whether `caller` may use the rule: an unlimited rule admits everyone,
    /// a limited one a caller on any of its lists.
    fn admits(&self, caller: Caller<'_>) -> bool {
        if self.service_accounts.is_none() && self.users.is_none() {
            return true;
        }
        on_list(self.service_accounts.as_deref(), caller.service_account)
            || on_list(self.users.as_deref(), caller.user)
    }
}

fn on_list(list: Option<&[String]>, name: Option<&str>) -> bool {
    list.zip(name)
        .is_some_and(|(list, name)| list.iter().any(|x| x == name))
}

/// An empty or blank list is refused: it would read as "everyone" to a
/// reviewer and match no one.
fn non_empty(list: Vec<String>, field: &str, path: &str) -> Result<Vec<String>, String> {
    if list.is_empty() || list.iter().any(|a| a.trim().is_empty()) {
        return Err(format!("rule `{path}` has an empty {field} entry or list"));
    }
    Ok(list)
}

/// An ordered allow list: the first rule whose method and path match and whose
/// limits admit the caller decides, and no match denies.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleSet(Vec<Rule>);

/// Why a [`RuleSet`] answered as it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Allowed by the rule with this path glob.
    Allow(String),
    /// A rule matched the method and path but its body rule did not.
    BodyRefused(String),
    /// A rule matched the method and path but its query rule did not.
    QueryRefused(String),
    /// No rule matched the method and path.
    NoRule,
}

impl RuleSet {
    pub fn new(rules: Vec<Rule>) -> Self {
        Self(rules)
    }

    /// Whether a request for `method` and `path` needs its body read before
    /// [`Self::check`] can answer.
    pub fn needs_body(&self, method: &str, path: &NormalPath, caller: Caller<'_>) -> bool {
        self.first(method, path, caller)
            .is_some_and(Rule::needs_body)
    }

    /// The verdict for one request from `caller`. `body` is
    /// `None` when it was streamed unread, which a rule with a body rule
    /// refuses.
    pub fn check(
        &self,
        method: &str,
        path: &NormalPath,
        caller: Caller<'_>,
        body: Option<&[u8]>,
    ) -> Verdict {
        match self.first(method, path, caller) {
            None => Verdict::NoRule,
            Some(rule)
                if rule
                    .query
                    .as_ref()
                    .is_some_and(|q| !q.matches(path.query())) =>
            {
                Verdict::QueryRefused(rule.path().to_string())
            }
            Some(rule) => match &rule.body {
                Some(body_rule) if !body.is_some_and(|b| body_rule.matches(b)) => {
                    Verdict::BodyRefused(rule.path().to_string())
                }
                _ => Verdict::Allow(rule.path().to_string()),
            },
        }
    }

    fn first(&self, method: &str, path: &NormalPath, caller: Caller<'_>) -> Option<&Rule> {
        self.0.iter().find(|rule| {
            rule.methods.iter().any(|m| m.eq_ignore_ascii_case(method))
                && rule.path.matches(path)
                && rule.admits(caller)
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
            rules.check("GET", &path("/repos/o/r/pulls/1"), Caller::default(), None),
            Verdict::Allow("/repos/**".to_string())
        );
        assert_eq!(
            rules.check("DELETE", &path("/repos/o/r"), Caller::default(), None),
            Verdict::NoRule
        );
        assert_eq!(
            rules.check("GET", &path("/user"), Caller::default(), None),
            Verdict::NoRule
        );
    }

    #[test]
    fn a_body_rule_reads_the_json_pointer() {
        let rules = rules();
        let reviews = path("/repos/o/r/pulls/1/reviews");
        assert!(rules.needs_body("POST", &reviews, Caller::default()));
        assert!(!rules.needs_body("GET", &reviews, Caller::default()));
        let check =
            |body: &str| rules.check("POST", &reviews, Caller::default(), Some(body.as_bytes()));
        assert!(matches!(check(r#"{"event":"COMMENT"}"#), Verdict::Allow(_)));
        assert!(matches!(check(r#"{"body":"pending"}"#), Verdict::Allow(_)));
        assert!(matches!(
            check(r#"{"event":"APPROVE"}"#),
            Verdict::BodyRefused(_)
        ));
        assert!(matches!(check("not json"), Verdict::BodyRefused(_)));
        assert!(matches!(
            rules.check("POST", &reviews, Caller::default(), None),
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
        let check = |p: &NormalPath, account| {
            let caller = Caller {
                service_account: account,
                user: Some("user-1"),
            };
            rules.check("GET", p, caller, None)
        };
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
    fn a_rule_naming_users_admits_them_on_any_service_account() {
        let rules = RuleSet::new(vec![
            Rule::new(&["GET"], "/prod/**", None)
                .unwrap()
                .for_service_accounts(vec!["moatless-ops".to_string()])
                .unwrap()
                .for_users(vec!["bot".to_string()])
                .unwrap(),
        ]);
        let prod = path("/prod/x");
        let check = |service_account, user| {
            rules.check(
                "GET",
                &prod,
                Caller {
                    service_account,
                    user,
                },
                None,
            )
        };
        assert!(matches!(
            check(Some("moatless-user"), Some("bot")),
            Verdict::Allow(_)
        ));
        assert!(matches!(check(None, Some("bot")), Verdict::Allow(_)));
        assert!(matches!(
            check(Some("moatless-ops"), Some("human")),
            Verdict::Allow(_)
        ));
        assert_eq!(check(Some("moatless-user"), Some("human")), Verdict::NoRule);
        assert_eq!(check(None, None), Verdict::NoRule);
    }

    #[test]
    fn a_query_rule_admits_only_one_listed_value() {
        let rules = RuleSet::new(vec![
            Rule::new(&["GET"], "/api/v1/query", None)
                .unwrap()
                .with_query(QueryRule {
                    param: "query".to_string(),
                    one_of: vec!["up{env=\"prod\"}".to_string()],
                })
                .unwrap(),
        ]);
        let check = |p: &str| rules.check("GET", &path(p), Caller::default(), None);
        assert!(matches!(
            check("/api/v1/query?query=up%7Benv%3D%22prod%22%7D&time=1"),
            Verdict::Allow(_)
        ));
        assert!(matches!(
            check("/api/v1/query?query=up{env=\"prod\"}"),
            Verdict::Allow(_)
        ));
        let refused = |p: &str| matches!(check(p), Verdict::QueryRefused(_));
        assert!(refused("/api/v1/query?query=up"));
        assert!(refused("/api/v1/query"));
        // A second copy could be the one the upstream reads.
        assert!(refused(
            "/api/v1/query?query=up%7Benv%3D%22prod%22%7D&query=secret"
        ));
        assert!(refused("/api/v1/query?query=%zz"));
        assert!(refused("/api/v1/query?query=%+f"));
        assert!(NormalPath::parse("/a%+2e").is_err());
        let spaced = QueryRule {
            param: "q".to_string(),
            one_of: vec!["a b".to_string()],
        };
        assert!(spaced.matches("q=a+b"));
        assert!(!spaced.matches("q=a%2Bb"));
    }

    #[test]
    fn a_query_rule_with_no_values_is_refused() {
        let spec: RuleSpec = serde_json::from_value(serde_json::json!({
            "methods": ["GET"], "path": "/x", "query": {"param": "q", "oneOf": []}
        }))
        .unwrap();
        assert!(Rule::from_spec(&spec).is_err());
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
        let spec: RuleSpec = serde_json::from_value(serde_json::json!({
            "methods": ["GET"], "path": "/x", "users": []
        }))
        .unwrap();
        assert!(Rule::from_spec(&spec).is_err());
    }
}
