//! Elicitation (`elicitation/create`): a server asking the user for input in the middle of a
//! request. The client hands the question to an [`ElicitationHandler`] and sends the answer
//! back as accept (with content), decline or cancel.
//!
//! The handler decides how the user is asked: the interactive mode shows a question, the other
//! modes decline. Without a handler a request is declined ([`DeclineAll`]).

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

/// A question from a server, as the handler sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum ElicitationRequest {
    /// Fill in a form. `schema` is the `requestedSchema` object (JSON Schema, object type with
    /// primitive properties).
    Form { message: String, schema: Value },
    /// Visit `url` in a browser. `elicitation_id` names the request.
    Url {
        message: String,
        url: String,
        elicitation_id: String,
    },
}

/// The user's answer. `Accept` carries the form content (`None` for a URL request, which has
/// none).
#[derive(Debug, Clone, PartialEq)]
pub enum ElicitationAnswer {
    Accept(Option<Value>),
    Decline,
    Cancel,
}

/// Asks the user on the server's behalf. `server` is the name the client was connected with.
///
/// Returns a boxed future so the trait needs no macro crate. Implementations may block inside
/// (on a thread from the caller's pool), as long as they do not block the runtime thread.
pub trait ElicitationHandler: Send + Sync + 'static {
    fn elicit<'a>(
        &'a self,
        server: &'a str,
        request: ElicitationRequest,
    ) -> Pin<Box<dyn Future<Output = ElicitationAnswer> + Send + 'a>>;
}

/// The handler used when none is given: every request is declined.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeclineAll;

impl ElicitationHandler for DeclineAll {
    fn elicit<'a>(
        &'a self,
        _server: &'a str,
        _request: ElicitationRequest,
    ) -> Pin<Box<dyn Future<Output = ElicitationAnswer> + Send + 'a>> {
        Box::pin(async { ElicitationAnswer::Decline })
    }
}
