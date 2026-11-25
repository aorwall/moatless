//! The one table naming every method this surface answers: its scope and
//! where it runs.
//!
//! [`crate::rpc::dispatch::methods`] merges each `dispatch/*.rs` group into the
//! table a connection answers from.

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

/// One method's entry.
pub(crate) struct Method {
    pub(crate) tag: &'static str,
    pub(crate) scope: Scope,
    /// `None` for a method the contract declares and this surface does not
    /// answer: see [`MethodTable::declared`].
    handler: Option<Handler>,
}

impl Method {
    /// What answers this method, if anything does.
    pub(crate) fn handler(&self) -> Option<&Handler> {
        self.handler.as_ref()
    }
}

/// Methods by tag, built one group at a time.
pub(crate) struct MethodTable {
    methods: BTreeMap<&'static str, Method>,
}

impl MethodTable {
    pub(crate) fn new() -> Self {
        Self {
            methods: BTreeMap::new(),
        }
    }

    pub(crate) fn detached<P, F, A>(self, tag: &'static str, scope: Scope, handler: F) -> Self
    where
        P: DeserializeOwned,
        F: Fn(Request<P>) -> A + Send + Sync + 'static,
        A: Future<Output = Vec<Value>> + Send + 'static,
    {
        self.register(tag, scope, Some(Handler::Detached(decoded(handler))))
    }

    pub(crate) fn session<P, F, A>(self, tag: &'static str, scope: Scope, handler: F) -> Self
    where
        P: DeserializeOwned,
        F: Fn(Request<P>) -> A + Send + Sync + 'static,
        A: Future<Output = Vec<Value>> + Send + 'static,
    {
        self.register(tag, scope, Some(Handler::Session(decoded(handler))))
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
        self.register(tag, scope, Some(Handler::Stream(open)))
    }

    /// A method the contract declares and this surface does not answer.
    ///
    /// Registered for its scope, so a withheld surface is refused for the scope
    /// rather than told it does not exist; only the first is actionable.
    pub(crate) fn declared(self, tag: &'static str, scope: Scope) -> Self {
        self.register(tag, scope, None)
    }

    /// Add every registration of `group`.
    ///
    /// Panics on a tag two groups both register: that is a table that cannot
    /// say what answers it.
    pub(crate) fn merge(mut self, group: MethodTable) -> Self {
        for (tag, method) in group.methods {
            self = self.register(tag, method.scope, method.handler);
        }
        self
    }

    fn register(mut self, tag: &'static str, scope: Scope, handler: Option<Handler>) -> Self {
        let previous = self.methods.insert(
            tag,
            Method {
                tag,
                scope,
                handler,
            },
        );
        assert!(previous.is_none(), "{tag} is registered twice");
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
    fn two_groups_registering_one_tag_are_refused() {
        let _ = MethodTable::new()
            .session("server.probe", Scope::OrchestrationRead, answer)
            .merge(MethodTable::new().declared("server.probe", Scope::OrchestrationRead));
    }
}
