//! Elicitation in the interactive mode: a server asks for input in the middle of an MCP tool
//! call, and the user answers in the TUI.
//!
//! The first question is a selector: Answer, Decline or Cancel. Answer opens the options pane
//! with one question per form field (choices for enums and booleans, free text otherwise). The
//! answers map back to the MCP response: a submitted form is accept with content, a skipped pane
//! is decline. A URL request shows the link and asks for Accept, Decline or Cancel.
//!
//! The question runs on a thread from the tools pool (`run_blocking`) and waits for the user
//! there, so the UI thread never blocks.

use std::future::Future;
use std::pin::Pin;

use hoocode_code_agent_session::mcp::{ElicitationAnswer, ElicitationHandler, ElicitationRequest};
use hoocode_code_permissions::PermissionUi;
use hoocode_code_tools_optin::{AskOption, AskOptionsHost, AskQuestion};
use hoocode_runtime::run_blocking;
use serde_json::{json, Map, Value};

use crate::dialog_bridge::{TuiAskOptionsHost, TuiPermissionUi};

const ANSWER: &str = "Answer";
const ACCEPT: &str = "Accept";
const DECLINE: &str = "Decline";
const CANCEL: &str = "Cancel";

/// The interactive mode's answer to elicitation requests.
pub struct TuiElicitation;

impl ElicitationHandler for TuiElicitation {
    fn elicit<'a>(
        &'a self,
        server: &'a str,
        request: ElicitationRequest,
    ) -> Pin<Box<dyn Future<Output = ElicitationAnswer> + Send + 'a>> {
        let server = server.to_owned();
        Box::pin(async move {
            run_blocking(move || ask(&server, request))
                .await
                .unwrap_or(ElicitationAnswer::Decline)
        })
    }
}

/// Asks the user, on the calling (blocking) thread.
fn ask(server: &str, request: ElicitationRequest) -> ElicitationAnswer {
    let ui = TuiPermissionUi;
    match request {
        ElicitationRequest::Url { message, url, .. } => {
            ui.notify(&format!(
                "MCP server {server}: {message}\nOpen this page to continue: {url}"
            ));
            let choice = ui.select(
                &format!("MCP server {server} asks you to open a page"),
                &[ACCEPT, DECLINE, CANCEL],
            );
            match choice.as_deref() {
                Some(ACCEPT) => ElicitationAnswer::Accept(None),
                Some(CANCEL) => ElicitationAnswer::Cancel,
                _ => ElicitationAnswer::Decline,
            }
        }
        ElicitationRequest::Form { message, schema } => {
            let choice = ui.select(
                &format!("MCP server {server} asks: {message}"),
                &[ANSWER, DECLINE, CANCEL],
            );
            match choice.as_deref() {
                Some(ANSWER) => {}
                Some(CANCEL) => return ElicitationAnswer::Cancel,
                _ => return ElicitationAnswer::Decline,
            }
            let fields = form_fields(&schema);
            let questions: Vec<AskQuestion> = fields.iter().map(Field::question).collect();
            let answers = if questions.is_empty() {
                Some(Vec::new())
            } else {
                TuiAskOptionsHost.ask_options(&questions, None)
            };
            match answers {
                Some(answers) => ElicitationAnswer::Accept(Some(content(&fields, &answers))),
                None => ElicitationAnswer::Decline,
            }
        }
    }
}

/// What the pane asks for one form field.
#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// Pick one: the label shown, and the JSON value it stands for.
    Choice(Vec<(String, Value)>),
    Text,
    Number {
        integer: bool,
    },
}

/// One property of the requested form.
#[derive(Debug, Clone, PartialEq)]
struct Field {
    key: String,
    title: String,
    detail: Option<String>,
    kind: Kind,
}

impl Field {
    fn question(&self) -> AskQuestion {
        let (options, allow_custom) = match &self.kind {
            Kind::Choice(choices) => (
                choices
                    .iter()
                    .map(|(label, _)| AskOption {
                        label: label.clone(),
                        description: None,
                        recommended: false,
                    })
                    .collect(),
                false,
            ),
            Kind::Text | Kind::Number { .. } => (Vec::new(), true),
        };
        AskQuestion {
            question: self.title.clone(),
            short: None,
            detail: self.detail.clone(),
            options,
            allow_custom,
        }
    }
}

/// The fields of a form schema (`properties`, in the order the server gave them).
fn form_fields(schema: &Value) -> Vec<Field> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    properties
        .iter()
        .map(|(key, property)| {
            let kind = match (
                enum_choices(property),
                property.get("type").and_then(Value::as_str),
            ) {
                (Some(choices), _) => Kind::Choice(choices),
                (None, Some("boolean")) => Kind::Choice(vec![
                    ("Yes".to_owned(), json!(true)),
                    ("No".to_owned(), json!(false)),
                ]),
                (None, Some("integer")) => Kind::Number { integer: true },
                (None, Some("number")) => Kind::Number { integer: false },
                _ => Kind::Text,
            };
            Field {
                key: key.clone(),
                title: property
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or(key)
                    .to_owned(),
                detail: property
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                kind,
            }
        })
        .collect()
}

/// The choices of an enum property: `oneOf` with `const` and `title`, or `enum` (with optional
/// `enumNames` labels).
fn enum_choices(property: &Value) -> Option<Vec<(String, Value)>> {
    if let Some(one_of) = property.get("oneOf").and_then(Value::as_array) {
        return Some(
            one_of
                .iter()
                .filter_map(|item| {
                    let value = item.get("const")?.clone();
                    let label = item
                        .get("title")
                        .and_then(Value::as_str)
                        .map_or_else(|| label_of(&value), str::to_owned);
                    Some((label, value))
                })
                .collect(),
        );
    }
    let values = property.get("enum").and_then(Value::as_array)?;
    let names = property.get("enumNames").and_then(Value::as_array);
    Some(
        values
            .iter()
            .enumerate()
            .map(|(i, value)| {
                let label = names
                    .and_then(|n| n.get(i))
                    .and_then(Value::as_str)
                    .map_or_else(|| label_of(value), str::to_owned);
                (label, value.clone())
            })
            .collect(),
    )
}

fn label_of(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

/// The form content from the pane's answers. A skipped field is left out; a choice goes back as
/// the value it stands for, and a number as a number.
fn content(fields: &[Field], answers: &[Option<String>]) -> Value {
    let mut content = Map::new();
    for (field, answer) in fields.iter().zip(answers) {
        let Some(text) = answer.as_deref() else {
            continue;
        };
        let value = match &field.kind {
            Kind::Choice(choices) => choices
                .iter()
                .find(|(label, _)| label == text)
                .map_or_else(|| json!(text), |(_, value)| value.clone()),
            Kind::Number { integer: true } => text
                .trim()
                .parse::<i64>()
                .map_or_else(|_| json!(text), |n| json!(n)),
            Kind::Number { integer: false } => text
                .trim()
                .parse::<f64>()
                .map_or_else(|_| json!(text), |n| json!(n)),
            Kind::Text => json!(text),
        };
        content.insert(field.key.clone(), value);
    }
    Value::Object(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "colour": {"type": "string", "title": "Colour", "enum": ["red", "blue"]},
                "size": {"type": "string", "oneOf": [
                    {"const": "s", "title": "Small"},
                    {"const": "l", "title": "Large"}
                ]},
                "agree": {"type": "boolean", "description": "Accept the terms"},
                "count": {"type": "integer"},
                "note": {"type": "string"}
            }
        })
    }

    #[test]
    fn fields_follow_the_schema_types() {
        let fields = form_fields(&schema());
        let keys: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            ["colour", "size", "agree", "count", "note"],
            "schema order"
        );
        let colour = fields.iter().find(|f| f.key == "colour").unwrap();
        assert_eq!(colour.title, "Colour");
        assert_eq!(
            colour.kind,
            Kind::Choice(vec![
                ("red".to_owned(), json!("red")),
                ("blue".to_owned(), json!("blue")),
            ])
        );
        let size = fields.iter().find(|f| f.key == "size").unwrap();
        assert_eq!(
            size.kind,
            Kind::Choice(vec![
                ("Small".to_owned(), json!("s")),
                ("Large".to_owned(), json!("l")),
            ])
        );
        let agree = fields.iter().find(|f| f.key == "agree").unwrap();
        assert_eq!(agree.detail.as_deref(), Some("Accept the terms"));
        assert_eq!(agree.question().options.len(), 2);
        assert!(!agree.question().allow_custom);
        let note = fields.iter().find(|f| f.key == "note").unwrap();
        assert!(note.question().allow_custom);
    }

    #[test]
    fn answers_become_the_form_content() {
        let fields = form_fields(&schema());
        // In the order of `fields` (the schema's order): colour, size, agree, count, note.
        let answers: Vec<Option<String>> = ["blue", "Large", "Yes", " 4 "]
            .into_iter()
            .map(|a| Some(a.to_owned()))
            .chain(std::iter::once(None))
            .collect();
        assert_eq!(
            content(&fields, &answers),
            json!({"agree": true, "colour": "blue", "count": 4, "size": "l"})
        );
    }

    #[test]
    fn a_schema_without_properties_has_no_questions() {
        assert!(form_fields(&json!({"type": "object"})).is_empty());
    }
}
