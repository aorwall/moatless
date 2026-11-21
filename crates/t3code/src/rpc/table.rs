//! The one table naming every method this surface answers: its scope, where it
//! runs, and which protocol answers it.
//!
//! [`crate::rpc::dispatch::methods`] merges each `dispatch/*.rs` group into the
//! table a connection answers from.

use crate::config::OrchestrationProtocol;
use crate::oauth::Scope;
use crate::rpc::envelope::RequestId;
use crate::rpc::envelope::failure_exit;
use crate::rpc::session::ProtocolSession;
use crate::rpc::session::SharedSession;
use crate::state::UiRpcState;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

/// One client request, owned by whatever answers it, carrying the payload its
/// method declares as `P`.
///
/// Owned, not borrowed, because a detached request outlives the frame-loop call
/// that routed it; every other field is an `Arc` or a `&'static str`, so owning
/// one costs only refcount bumps.
pub(crate) struct Request<P> {
    pub(crate) id: RequestId,
    /// The method's tag, kept because the failure frames echo it back.
    pub(crate) tag: &'static str,
    /// The scope the method's entry requires, which a per-Task refusal names.
    pub(crate) scope: Scope,
    pub(crate) payload: P,
    pub(crate) state: UiRpcState,
    pub(crate) identity: UiRpcAuthIdentity,
    pub(crate) shared: SharedSession,
}

impl Request<Value> {
    /// The request with its payload decoded as `P`, or the `Die` that refuses
    /// a payload `P` does not decode.
    ///
    /// An absent payload decodes as `{}`, so it answers as one that sends no
    /// field rather than as one of the wrong type.
    fn decode<P: DeserializeOwned>(self) -> Result<Request<P>, Vec<Value>> {
        let Request {
            id,
            tag,
            scope,
            payload,
            state,
            identity,
            shared,
        } = self;
        let payload = match payload {
            Value::Null => Value::Object(serde_json::Map::new()),
            payload => payload,
        };
        match serde_json::from_value(payload) {
            Ok(payload) => Ok(Request {
                id,
                tag,
                scope,
                payload,
                state,
                identity,
                shared,
            }),
            Err(error) => Err(vec![failure_exit(&id, format!("{tag}: {error}"))]),
        }
    }
}

/// The frames a request answers with, once it finishes.
pub(crate) type Answer<'a> = Pin<Box<dyn Future<Output = Vec<Value>> + Send + 'a>>;

type Respond = Box<dyn Fn(Request<Value>) -> Answer<'static> + Send + Sync>;

type Open =
    Box<dyn for<'a> Fn(&'a mut ProtocolSession, Request<Value>) -> Answer<'a> + Send + Sync>;

/// Where a method runs, and what it may reach while it does.
pub(crate) enum Handler {
    /// Off the frame loop, at most
    /// [`crate::rpc::session::MAX_DETACHED_REQUESTS`] at once. It may see a
    /// transition sent after it, answers in no fixed order, and locks
    /// [`SharedSession`] per read or write, never across an await.
    Detached(Respond),
    /// On the frame loop: the loop finishes it before dispatching anything sent
    /// after it, the guarantee a detached read relies on. Every transition is
    /// one.
    Session(Respond),
    /// [`Self::Session`], holding the connection it opens a stream on.
    Stream(Open),
}

impl Handler {
    #[cfg(test)]
    pub(crate) fn mode(&self) -> &'static str {
        match self {
            Handler::Detached(_) => "detached",
            Handler::Session(_) => "session",
            Handler::Stream(_) => "stream",
        }
    }
}

/// `handler` behind the decode of its payload.
fn decoded<P, F, A>(handler: F) -> Respond
where
    P: DeserializeOwned,
    F: Fn(Request<P>) -> A + Send + Sync + 'static,
    A: Future<Output = Vec<Value>> + Send + 'static,
{
    Box::new(move |request| -> Answer<'static> {
        match request.decode() {
            Ok(request) => Box::pin(handler(request)),
            Err(died) => Box::pin(std::future::ready(died)),
        }
    })
}

/// Pins a stream closure's signature, which a closure only takes from a bound.
fn opening<F>(open: F) -> F
where
    F: for<'a> Fn(&'a mut ProtocolSession, Request<Value>) -> Answer<'a>,
{
    open
}

/// What one group registered for a method.
enum Registration {
    Answered(Handler),
    /// Answers `UnsupportedMethodError` once the scope check passes.
    Declared,
}

/// One method's entry.
pub(crate) struct Method {
    pub(crate) tag: &'static str,
    pub(crate) scope: Scope,
    /// Registered by a [`MethodTable::new`] group: answers both protocols.
    any: Option<Registration>,
    /// Registered by a [`MethodTable::v2`] group: replaces `any` on a V2
    /// connection.
    v2: Option<Registration>,
}

impl Method {
    /// What answers this method on a connection speaking `protocol`.
    pub(crate) fn handler(&self, protocol: OrchestrationProtocol) -> Option<&Handler> {
        let registered = match protocol {
            OrchestrationProtocol::V1 => self.any.as_ref(),
            OrchestrationProtocol::V2 => self.v2.as_ref().or(self.any.as_ref()),
        };
        match registered {
            Some(Registration::Answered(handler)) => Some(handler),
            Some(Registration::Declared) | None => None,
        }
    }

    /// Whether a V2 connection is answered by its own registration.
    #[cfg(test)]
    pub(crate) fn v2_is_own(&self) -> bool {
        self.v2.is_some()
    }
}

/// Which protocol a group's registrations answer.
#[derive(Clone, Copy)]
enum Since {
    Any,
    V2,
}

/// Methods by tag, built one group at a time.
pub(crate) struct MethodTable {
    since: Since,
    methods: BTreeMap<&'static str, Method>,
}

impl MethodTable {
    /// A group whose methods answer both protocols.
    pub(crate) fn new() -> Self {
        Self {
            since: Since::Any,
            methods: BTreeMap::new(),
        }
    }

    /// A group whose methods answer a V2 connection only, replacing a
    /// [`Self::new`] group's registration of the same tag there.
    pub(crate) fn v2() -> Self {
        Self {
            since: Since::V2,
            ..Self::new()
        }
    }

    pub(crate) fn detached<P, F, A>(self, tag: &'static str, scope: Scope, handler: F) -> Self
    where
        P: DeserializeOwned,
        F: Fn(Request<P>) -> A + Send + Sync + 'static,
        A: Future<Output = Vec<Value>> + Send + 'static,
    {
        let respond = decoded(handler);
        let since = self.since;
        self.register(
            tag,
            scope,
            since,
            Registration::Answered(Handler::Detached(respond)),
        )
    }

    pub(crate) fn session<P, F, A>(self, tag: &'static str, scope: Scope, handler: F) -> Self
    where
        P: DeserializeOwned,
        F: Fn(Request<P>) -> A + Send + Sync + 'static,
        A: Future<Output = Vec<Value>> + Send + 'static,
    {
        let respond = decoded(handler);
        let since = self.since;
        self.register(
            tag,
            scope,
            since,
            Registration::Answered(Handler::Session(respond)),
        )
    }

    pub(crate) fn stream<P: DeserializeOwned + 'static>(
        self,
        tag: &'static str,
        scope: Scope,
        handler: for<'a> fn(&'a mut ProtocolSession, Request<P>) -> Answer<'a>,
    ) -> Self {
        let open: Open = Box::new(opening(move |session, request| match request.decode() {
            Ok(request) => handler(session, request),
            Err(died) => Box::pin(std::future::ready(died)),
        }));
        let since = self.since;
        self.register(
            tag,
            scope,
            since,
            Registration::Answered(Handler::Stream(open)),
        )
    }

    /// A method the contract declares and this group does not answer.
    ///
    /// Registered for its scope, so a withheld surface is refused for the scope
    /// rather than told it does not exist; only the first is actionable.
    pub(crate) fn declared(self, tag: &'static str, scope: Scope) -> Self {
        let since = self.since;
        self.register(tag, scope, since, Registration::Declared)
    }

    /// Add every registration of `group`.
    ///
    /// Panics on a tag two groups register for the same protocol, or register
    /// under two scopes: either is a table that cannot say what answers it.
    pub(crate) fn merge(mut self, group: MethodTable) -> Self {
        for (tag, method) in group.methods {
            if let Some(any) = method.any {
                self = self.register(tag, method.scope, Since::Any, any);
            }
            if let Some(v2) = method.v2 {
                self = self.register(tag, method.scope, Since::V2, v2);
            }
        }
        self
    }

    fn register(
        mut self,
        tag: &'static str,
        scope: Scope,
        since: Since,
        registration: Registration,
    ) -> Self {
        let method = self.methods.entry(tag).or_insert(Method {
            tag,
            scope,
            any: None,
            v2: None,
        });
        assert!(
            method.scope == scope,
            "{tag} is registered under {} and {scope}",
            method.scope
        );
        let slot = match since {
            Since::Any => &mut method.any,
            Since::V2 => &mut method.v2,
        };
        assert!(slot.is_none(), "{tag} is registered twice");
        *slot = Some(registration);
        self
    }

    pub(crate) fn get(&self, tag: &str) -> Option<&Method> {
        self.methods.get(tag)
    }

    #[cfg(test)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Method> {
        self.methods.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn answer(_: Request<Value>) -> Vec<Value> {
        Vec::new()
    }

    #[test]
    #[should_panic(expected = "registered twice")]
    fn two_groups_answering_one_tag_for_the_same_protocol_are_refused() {
        let _ = MethodTable::new()
            .session("server.probe", Scope::OrchestrationRead, answer)
            .merge(MethodTable::new().detached("server.probe", Scope::OrchestrationRead, answer));
    }

    #[test]
    #[should_panic(expected = "registered under")]
    fn one_tag_under_two_scopes_is_refused() {
        let _ = MethodTable::new()
            .session("vcs.switchRef", Scope::OrchestrationOperate, answer)
            .merge(MethodTable::v2().declared("vcs.switchRef", Scope::OrchestrationRead));
    }

    /// A V2 registration replaces the shared one on V2 only, and a V2-only
    /// tag is absent on V1.
    #[test]
    fn a_v2_group_answers_v2_connections_only() {
        let table = MethodTable::new()
            .session("test.replay", Scope::OrchestrationRead, answer)
            .merge(
                MethodTable::v2()
                    .declared("test.replay", Scope::OrchestrationRead)
                    .detached(
                        "orchestration.getTurnItem",
                        Scope::OrchestrationRead,
                        answer,
                    ),
            );
        let replay = table.get("test.replay").expect("registered");
        assert!(replay.handler(OrchestrationProtocol::V1).is_some());
        assert!(replay.handler(OrchestrationProtocol::V2).is_none());
        let turn_item = table.get("orchestration.getTurnItem").expect("registered");
        assert!(turn_item.handler(OrchestrationProtocol::V1).is_none());
        assert_eq!(
            turn_item
                .handler(OrchestrationProtocol::V2)
                .map(Handler::mode),
            Some("detached")
        );
    }
}
