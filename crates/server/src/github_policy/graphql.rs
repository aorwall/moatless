//! Which GraphQL operations a sandbox may send.
//!
//! Every operation in the document is checked, not only `operationName`:
//! GitHub runs the one named, but a caller choosing the name must not be able
//! to hide a mutation behind a query.

use std::collections::{HashMap, HashSet};

use apollo_parser::Parser;
use apollo_parser::cst;
use moatless_core::workspace_identity_grants::GitHubCapability;
use moatless_workspace_types::repository::GITHUB_HOST;
use serde::Deserialize;
use serde_json::Value;

use super::refusal::{Refusal, capability_rule, review_event_approves};
use super::scope::{PolicyScope, repo_key};

/// Mutations a sandbox may send with no capability. Each names its subject by
/// node id, so only the variables' `owner`/`repo`, when present, are checked
/// against the Workspace's repositories.
const ALLOWED_MUTATIONS: &[&str] = &[
    "addAssigneesToAssignable",
    "addComment",
    "addLabelsToLabelable",
    "addPullRequestReviewComment",
    "addPullRequestReviewThread",
    "addPullRequestReviewThreadReply",
    "addReaction",
    "closeIssue",
    "closePullRequest",
    "convertPullRequestToDraft",
    "createIssue",
    "createPullRequest",
    "deleteIssueComment",
    "deletePullRequestReview",
    "markPullRequestReadyForReview",
    "minimizeComment",
    "removeAssigneesFromAssignable",
    "removeLabelsFromLabelable",
    "removeReaction",
    "reopenIssue",
    "reopenPullRequest",
    "requestReviews",
    "resolveReviewThread",
    "unresolveReviewThread",
    "updateIssue",
    "updateIssueComment",
    "updatePullRequest",
    "updatePullRequestReview",
    "updatePullRequestReviewComment",
];

/// Mutations whose `input.event` may approve.
const REVIEW_MUTATIONS: &[&str] = &["addPullRequestReview", "submitPullRequestReview"];

/// Bounds the work one parse does; `gh`'s largest documents are far below.
const TOKEN_LIMIT: usize = 20_000;
const RECURSION_LIMIT: usize = 100;

#[derive(Deserialize)]
struct Envelope {
    query: String,
    #[serde(default)]
    variables: Option<Value>,
}

/// One root field of a mutation, as the policy sees it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Mutation {
    pub name: String,
    /// For a review mutation, whether its event approves or cannot be read.
    pub approves: bool,
}

/// What a GraphQL request does, read from every operation in it.
#[derive(Debug, Default)]
pub(super) struct Document {
    mutations: Vec<Mutation>,
    subscription: bool,
    /// The repository the variables name as `owner` with `repo` or `name`.
    repository: Option<(String, String)>,
}

impl Document {
    /// Parses a GraphQL request body. A body that is no `{query}` envelope, or
    /// a document that does not parse and names `mutation` or `subscription`,
    /// is refused: a policy that cannot read a write cannot vouch for it.
    /// Unparsable text with neither keyword cannot be a write, so it is a read.
    pub(super) fn parse(body: &[u8]) -> Result<Self, Refusal> {
        let unparsable = |why: String| {
            Refusal::new(
                "graphql_unparsable",
                format!("The GitHub proxy could not read this GraphQL request: {why}"),
            )
        };
        let envelope: Envelope =
            serde_json::from_slice(body).map_err(|e| unparsable(e.to_string()))?;
        let may_write = ["mutation", "subscription"]
            .iter()
            .any(|keyword| envelope.query.contains(keyword));
        let unparsable = |why: String| {
            if may_write {
                Err(unparsable(why))
            } else {
                Ok(Self::default())
            }
        };
        let tree = Parser::new(&envelope.query)
            .token_limit(TOKEN_LIMIT)
            .recursion_limit(RECURSION_LIMIT)
            .parse();
        if let Some(error) = tree.errors().next() {
            return unparsable(error.message().to_string());
        }
        let variables = envelope.variables.unwrap_or(Value::Null);
        let document = tree.document();

        let fragments: HashMap<String, cst::SelectionSet> = document
            .definitions()
            .filter_map(|definition| match definition {
                cst::Definition::FragmentDefinition(fragment) => Some((
                    fragment.fragment_name()?.name()?.text().to_string(),
                    fragment.selection_set()?,
                )),
                _ => None,
            })
            .collect();

        let mut out = Self {
            repository: repository(&variables),
            ..Self::default()
        };
        for definition in document.definitions() {
            let operation = match definition {
                cst::Definition::OperationDefinition(operation) => operation,
                cst::Definition::FragmentDefinition(_) => continue,
                _ => return unparsable("a type system definition".to_string()),
            };
            let kind = operation.operation_type();
            if kind
                .as_ref()
                .is_some_and(|k| k.subscription_token().is_some())
            {
                out.subscription = true;
                continue;
            }
            if kind.as_ref().is_none_or(|k| k.mutation_token().is_none()) {
                continue;
            }
            let Some(selections) = operation.selection_set() else {
                continue;
            };
            let mut fields = Vec::new();
            root_fields(&selections, &fragments, &mut HashSet::new(), &mut fields);
            for field in fields {
                let Some(name) = field.name().map(|n| n.text().to_string()) else {
                    continue;
                };
                let approves = REVIEW_MUTATIONS.contains(&name.as_str())
                    && review_event_approves(review_event(&field, &variables).as_ref());
                out.mutations.push(Mutation { name, approves });
            }
        }
        Ok(out)
    }

    /// The root field of every mutation in the document.
    pub(super) fn mutation_names(&self) -> impl Iterator<Item = &str> {
        self.mutations.iter().map(|m| m.name.as_str())
    }

    /// Whether the document holds anything but queries.
    pub(super) fn writes(&self) -> bool {
        !self.mutations.is_empty() || self.subscription
    }
}

/// The root fields of `selections`, with fragment spreads and inline
/// fragments expanded; `seen` stops a fragment cycle.
fn root_fields(
    selections: &cst::SelectionSet,
    fragments: &HashMap<String, cst::SelectionSet>,
    seen: &mut HashSet<String>,
    out: &mut Vec<cst::Field>,
) {
    for selection in selections.selections() {
        match selection {
            cst::Selection::Field(field) => out.push(field),
            cst::Selection::InlineFragment(inline) => {
                if let Some(inner) = inline.selection_set() {
                    root_fields(&inner, fragments, seen, out);
                }
            }
            cst::Selection::FragmentSpread(spread) => {
                let Some(name) = spread
                    .fragment_name()
                    .and_then(|n| n.name())
                    .map(|n| n.text().to_string())
                else {
                    continue;
                };
                if let Some(inner) = fragments.get(&name) {
                    if seen.insert(name) {
                        root_fields(inner, fragments, seen, out);
                    }
                }
            }
        }
    }
}

/// The `input.event` a review mutation sends: from the literal, or from the
/// variable the literal or the whole `input` names. `None` when no event is
/// sent; `Some(Value::Bool(true))` stands for one that cannot be read.
fn review_event(field: &cst::Field, variables: &Value) -> Option<Value> {
    let unreadable = Some(Value::Bool(true));
    let Some(input) = field
        .arguments()
        .into_iter()
        .flat_map(|arguments| arguments.arguments())
        .find(|argument| argument.name().is_some_and(|n| n.text() == "input"))
        .and_then(|argument| argument.value())
    else {
        return unreadable;
    };
    match input {
        cst::Value::Variable(variable) => {
            let name = variable.name()?.text().to_string();
            match variables.get(&name) {
                Some(Value::Object(input)) => input.get("event").cloned(),
                _ => unreadable,
            }
        }
        cst::Value::ObjectValue(object) => {
            let event = object
                .object_fields()
                .find(|f| f.name().is_some_and(|n| n.text() == "event"))
                .and_then(|f| f.value());
            match event {
                None => None,
                Some(cst::Value::EnumValue(value)) => value
                    .name()
                    .map(|n| Value::String(n.text().to_string()))
                    .or(unreadable),
                Some(cst::Value::StringValue(value)) => Some(Value::String(String::from(value))),
                Some(cst::Value::Variable(variable)) => variable
                    .name()
                    .and_then(|n| variables.get(n.text().as_str()).cloned())
                    .or(unreadable),
                Some(_) => unreadable,
            }
        }
        _ => unreadable,
    }
}

/// The `owner` with `repo` or `name` the variables name, as `gh` sends them.
fn repository(variables: &Value) -> Option<(String, String)> {
    let owner = variables.get("owner")?.as_str()?;
    let name = variables
        .get("repo")
        .or_else(|| variables.get("name"))?
        .as_str()?;
    Some((owner.to_string(), name.to_string()))
}

/// `scope` is `None` only for a document that [`Document::writes`] nothing.
pub(super) fn check(document: &Document, scope: Option<&PolicyScope>) -> Result<(), Refusal> {
    if document.subscription {
        return Err(Refusal::new(
            "graphql_subscription",
            "The GitHub proxy forwards no GraphQL subscription",
        ));
    }
    let Some(scope) = scope else {
        return Ok(());
    };
    if document.mutations.is_empty() {
        return Ok(());
    }
    if let Some((owner, name)) = &document.repository {
        if !scope.allows(&repo_key(GITHUB_HOST, owner, name)) {
            return Err(Refusal::new(
                "repo_scope",
                format!(
                    "{owner}/{name} is not one of this workspace's repositories: place it in the \
                     workspace, or add it to `githubPolicy.extraRepos` in its \
                     WORKSPACE_IDENTITY_GRANTS entry"
                ),
            ));
        }
    }
    for mutation in &document.mutations {
        let name = mutation.name.as_str();
        let needs = match name {
            _ if ALLOWED_MUTATIONS.contains(&name) => None,
            _ if REVIEW_MUTATIONS.contains(&name) => {
                mutation.approves.then_some(GitHubCapability::Approve)
            }
            "mergePullRequest" => Some(GitHubCapability::Merge),
            _ => {
                return Err(Refusal::new(
                    "graphql_mutation",
                    format!("The GitHub proxy forwards no `{name}` mutation"),
                ));
            }
        };
        if let Some(capability) = needs.filter(|c| !scope.has(*c)) {
            return Err(Refusal::new(
                capability_rule(capability),
                format!(
                    "`{name}` needs the `{}` capability in `githubPolicy.capabilities` of this \
                     workspace's WORKSPACE_IDENTITY_GRANTS entry",
                    capability.as_str()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::github_policy::test_support::scope;

    fn body(query: &str, variables: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({ "query": query, "variables": variables })).unwrap()
    }

    fn decide(
        capabilities: &[GitHubCapability],
        query: &str,
        variables: Value,
    ) -> Result<(), &'static str> {
        let document = Document::parse(&body(query, variables)).map_err(|r| r.rule)?;
        let scope = scope(capabilities);
        check(&document, document.writes().then_some(&scope)).map_err(|r| r.rule)
    }

    #[test]
    fn a_query_on_any_repository_is_allowed() {
        assert_eq!(
            decide(
                &[],
                "query($owner: String!, $repo: String!) { repository(owner: $owner, name: $repo) { id } }",
                json!({ "owner": "elsewhere", "repo": "x" })
            ),
            Ok(())
        );
        assert_eq!(decide(&[], "{ viewer { login } }", Value::Null), Ok(()));
    }

    #[test]
    fn an_allowed_mutation_passes_and_an_unknown_one_is_refused() {
        let resolve =
            "mutation($id: ID!) { resolveReviewThread(input: {threadId: $id}) { thread { id } } }";
        assert_eq!(decide(&[], resolve, json!({ "id": "T_1" })), Ok(()));
        let delete =
            "mutation { deleteRepository(input: {repositoryId: \"R\"}) { clientMutationId } }";
        assert_eq!(decide(&[], delete, Value::Null), Err("graphql_mutation"));
        let auto = "mutation { enablePullRequestAutoMerge(input: {pullRequestId: \"P\"}) { clientMutationId } }";
        assert_eq!(
            decide(&[GitHubCapability::Merge], auto, Value::Null),
            Err("graphql_mutation")
        );
    }

    #[test]
    fn a_mutation_hidden_behind_another_operation_name_or_a_fragment_is_found() {
        let two = "query Safe { viewer { login } } \
                   mutation Evil { deleteRef(input: {refId: \"R\"}) { clientMutationId } }";
        assert_eq!(decide(&[], two, Value::Null), Err("graphql_mutation"));
        let spread = "mutation { ...M } fragment M on Mutation { deleteRef(input: {refId: \"R\"}) { clientMutationId } }";
        assert_eq!(decide(&[], spread, Value::Null), Err("graphql_mutation"));
        let inline = "mutation { ... on Mutation { deleteRef(input: {refId: \"R\"}) { clientMutationId } } }";
        assert_eq!(decide(&[], inline, Value::Null), Err("graphql_mutation"));
        let aliased = "mutation { ok: deleteRef(input: {refId: \"R\"}) { clientMutationId } }";
        assert_eq!(decide(&[], aliased, Value::Null), Err("graphql_mutation"));
    }

    #[test]
    fn an_approving_review_needs_the_approve_capability_however_the_event_is_sent() {
        let by_variable = "mutation($input: AddPullRequestReviewInput!) { addPullRequestReview(input: $input) { clientMutationId } }";
        let approve = json!({ "input": { "pullRequestId": "P", "event": "APPROVE" } });
        assert_eq!(decide(&[], by_variable, approve.clone()), Err("approve"));
        assert_eq!(
            decide(&[GitHubCapability::Approve], by_variable, approve),
            Ok(())
        );
        assert_eq!(
            decide(
                &[],
                by_variable,
                json!({ "input": { "pullRequestId": "P", "event": "COMMENT" } })
            ),
            Ok(())
        );
        assert_eq!(
            decide(
                &[],
                by_variable,
                json!({ "input": { "pullRequestId": "P" } })
            ),
            Ok(())
        );
        assert_eq!(decide(&[], by_variable, json!({})), Err("approve"));

        let literal = "mutation { submitPullRequestReview(input: {pullRequestReviewId: \"R\", event: APPROVE}) { clientMutationId } }";
        assert_eq!(decide(&[], literal, Value::Null), Err("approve"));
        let inner_variable = "mutation($e: PullRequestReviewEvent!) { submitPullRequestReview(input: {pullRequestReviewId: \"R\", event: $e}) { clientMutationId } }";
        assert_eq!(
            decide(&[], inner_variable, json!({ "e": "APPROVE" })),
            Err("approve")
        );
        assert_eq!(
            decide(&[], inner_variable, json!({ "e": "COMMENT" })),
            Ok(())
        );
        assert_eq!(decide(&[], inner_variable, json!({})), Err("approve"));
    }

    #[test]
    fn a_merge_needs_the_merge_capability() {
        let merge =
            "mutation { mergePullRequest(input: {pullRequestId: \"P\"}) { clientMutationId } }";
        assert_eq!(decide(&[], merge, Value::Null), Err("merge"));
        assert_eq!(
            decide(&[GitHubCapability::Merge], merge, Value::Null),
            Ok(())
        );
    }

    #[test]
    fn a_mutation_naming_another_repository_in_its_variables_is_refused() {
        let create = "mutation($owner: String!, $repo: String!) { createIssue(input: {repositoryId: \"R\", title: \"t\"}) { clientMutationId } }";
        assert_eq!(
            decide(&[], create, json!({ "owner": "soaplabs", "repo": "other" })),
            Err("repo_scope")
        );
        assert_eq!(
            decide(
                &[],
                create,
                json!({ "owner": "soaplabs", "repo": "moatless" })
            ),
            Ok(())
        );
    }

    #[test]
    fn a_subscription_or_an_unparsable_document_is_refused() {
        assert_eq!(
            decide(&[], "subscription { x }", Value::Null),
            Err("graphql_subscription")
        );
        assert_eq!(
            decide(&[], "mutation {", Value::Null),
            Err("graphql_unparsable")
        );
        assert_eq!(
            Document::parse(b"not json").map(|_| ()).map_err(|r| r.rule),
            Err("graphql_unparsable")
        );
    }

    #[test]
    fn an_unparsable_document_naming_no_write_keyword_is_a_read() {
        assert_eq!(decide(&[], "query { viewer {", Value::Null), Ok(()));
        assert_eq!(decide(&[], "type Query { x: Int }", Value::Null), Ok(()));
        let long = format!("query {{ {} }}", "a ".repeat(TOKEN_LIMIT + 1));
        assert_eq!(decide(&[], &long, Value::Null), Ok(()));
    }
}
