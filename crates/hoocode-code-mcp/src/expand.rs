//! `${VAR}` and `${VAR:-default}` expansion in `mcp.json` values (`docs/design/mcp.md`).
//!
//! Expansion happens when a server is about to start, never when the file is read. The trust
//! fingerprint and the diagnostics therefore see the text as written, so rotating a token in the
//! environment does not ask for trust again. Expanded values are secrets: they are returned to
//! the caller and never printed. Errors name the field and the variable, never the value.
//!
//! Applies to `command`, `args`, `env` values, `url` and `headers` values. `cwd` is kept as
//! written. A `${NAME}` whose variable is unset, with no default, is an error; a server with one
//! is not started. `${NAME:-default}` uses the default when the variable is unset or empty.

use std::fmt;

use crate::config::Transport;

/// A value that cannot be expanded. `field` says where it is (`headers.Authorization`, not its
/// value), and `message` says why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandError {
    pub field: String,
    pub message: String,
}

impl fmt::Display for ExpandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl std::error::Error for ExpandError {}

/// Expands every `${...}` in `text` with `lookup`. `field` names the value in errors.
pub fn expand_value(
    field: &str,
    text: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<String, ExpandError> {
    expand_into(field, text, lookup, &mut Vec::new())
}

/// [`expand_value`], also pushing each non-empty value a variable expanded to onto `used`.
fn expand_into(
    field: &str,
    text: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
    used: &mut Vec<String>,
) -> Result<String, ExpandError> {
    let fail = |message: String| ExpandError {
        field: field.to_owned(),
        message,
    };
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find('}')
            .ok_or_else(|| fail("unterminated \"${\" (missing \"}\")".to_owned()))?;
        let inner = &after[..end];
        let (name, default) = match inner.split_once(":-") {
            Some((name, default)) => (name, Some(default)),
            None => (inner, None),
        };
        if !is_var_name(name) {
            return Err(fail(format!(
                "\"${{{inner}}}\" is not a variable name (letters, digits and _, not starting with a digit)"
            )));
        }
        match (lookup(name), default) {
            (Some(value), Some(default)) if value.is_empty() => out.push_str(default),
            (Some(value), _) => {
                if !value.is_empty() {
                    used.push(value.clone());
                }
                out.push_str(&value);
            }
            (None, Some(default)) => out.push_str(default),
            (None, None) => {
                return Err(fail(format!(
                    "environment variable {name} is not set; set it, or write ${{{name}:-default}}"
                )))
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn is_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

impl Transport {
    /// This transport with every expandable value expanded through `lookup`. The parsed
    /// transport is not changed, so the trust fingerprint still sees the text as written.
    ///
    /// An HTTP `url` is checked again after expansion: it must start with `http://` or
    /// `https://`, which parsing could not check while it still held a `${...}`.
    pub fn expanded(
        &self,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Transport, ExpandError> {
        self.expanded_values(lookup).map(|(transport, _)| transport)
    }

    /// [`expanded`](Self::expanded), also returning every non-empty value a variable expanded
    /// to. Those are secrets: keep them out of anything shown or logged.
    pub fn expanded_values(
        &self,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<(Transport, Vec<String>), ExpandError> {
        let mut used = Vec::new();
        let transport = match self {
            Transport::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                let command = expand_into("command", command, lookup, &mut used)?;
                let args = args
                    .iter()
                    .enumerate()
                    .map(|(i, arg)| expand_into(&format!("args[{i}]"), arg, lookup, &mut used))
                    .collect::<Result<Vec<_>, _>>()?;
                let env = env
                    .iter()
                    .map(|(k, v)| {
                        Ok((
                            k.clone(),
                            expand_into(&format!("env.{k}"), v, lookup, &mut used)?,
                        ))
                    })
                    .collect::<Result<_, ExpandError>>()?;
                Transport::Stdio {
                    command,
                    args,
                    env,
                    cwd: cwd.clone(),
                }
            }
            Transport::Http { url, headers } => {
                let url = expand_into("url", url, lookup, &mut used)?;
                let lower = url.to_ascii_lowercase();
                if !(lower.starts_with("http://") || lower.starts_with("https://")) {
                    return Err(ExpandError {
                        field: "url".to_owned(),
                        message: "must start with http:// or https:// once expanded".to_owned(),
                    });
                }
                let headers = headers
                    .iter()
                    .map(|(k, v)| {
                        Ok((
                            k.clone(),
                            expand_into(&format!("headers.{k}"), v, lookup, &mut used)?,
                        ))
                    })
                    .collect::<Result<_, ExpandError>>()?;
                Transport::Http { url, headers }
            }
        };
        Ok((transport, used))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name: &str| map.get(name).cloned()
    }

    #[test]
    fn expands_plain_and_defaulted_variables() {
        let lookup = env(&[("TOKEN", "abc"), ("EMPTY", "")]);
        let run = |text| expand_value("f", text, &lookup).unwrap();
        assert_eq!(run("Bearer ${TOKEN}"), "Bearer abc");
        assert_eq!(run("${TOKEN}${TOKEN}"), "abcabc");
        assert_eq!(run("${MISSING:-fallback}"), "fallback");
        assert_eq!(run("${TOKEN:-fallback}"), "abc");
        assert_eq!(
            run("${EMPTY:-fallback}"),
            "fallback",
            "empty counts as unset with a default"
        );
        assert_eq!(
            run("${EMPTY}"),
            "",
            "a set but empty variable expands to nothing"
        );
        assert_eq!(run("${MISSING:-}"), "");
        assert_eq!(run("no variables $HOME here"), "no variables $HOME here");
    }

    #[test]
    fn an_unset_variable_without_default_names_it_and_not_its_value() {
        let lookup = env(&[("SECRET_TOKEN_VALUE", "hunter2")]);
        let err =
            expand_value("headers.Authorization", "Bearer ${GITHUB_TOKEN}", &lookup).unwrap_err();
        assert_eq!(err.field, "headers.Authorization");
        let text = err.to_string();
        assert!(text.contains("GITHUB_TOKEN"), "{text}");
        assert!(text.contains("headers.Authorization"), "{text}");
    }

    #[test]
    fn malformed_references_are_errors() {
        let lookup = env(&[]);
        assert!(expand_value("f", "${OPEN", &lookup).is_err());
        assert!(expand_value("f", "${}", &lookup).is_err());
        assert!(expand_value("f", "${1ABC:-x}", &lookup).is_err());
        assert!(expand_value("f", "${A B}", &lookup).is_err());
    }

    #[test]
    fn every_expandable_field_is_expanded() {
        let lookup = env(&[
            ("CMD", "npx"),
            ("ARG", "pkg"),
            ("KEY", "k1"),
            ("BASE", "https://example.com"),
            ("TOKEN", "t1"),
        ]);
        let stdio = Transport::Stdio {
            command: "${CMD}".into(),
            args: vec!["-y".into(), "${ARG}".into()],
            env: BTreeMap::from([("API_KEY".to_owned(), "${KEY}".to_owned())]),
            cwd: Some("${CMD}".into()),
        };
        assert_eq!(
            stdio.expanded(&lookup).unwrap(),
            Transport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "pkg".into()],
                env: BTreeMap::from([("API_KEY".to_owned(), "k1".to_owned())]),
                // cwd is kept as written.
                cwd: Some("${CMD}".into()),
            }
        );
        let http = Transport::Http {
            url: "${BASE}/mcp".into(),
            headers: BTreeMap::from([("Authorization".to_owned(), "Bearer ${TOKEN}".to_owned())]),
        };
        assert_eq!(
            http.expanded(&lookup).unwrap(),
            Transport::Http {
                url: "https://example.com/mcp".into(),
                headers: BTreeMap::from([("Authorization".to_owned(), "Bearer t1".to_owned())]),
            }
        );
    }

    #[test]
    fn a_missing_variable_in_any_field_fails_the_transport() {
        let lookup = env(&[]);
        let stdio = Transport::Stdio {
            command: "x".into(),
            args: vec!["${NOPE}".into()],
            env: BTreeMap::new(),
            cwd: None,
        };
        let err = stdio.expanded(&lookup).unwrap_err();
        assert_eq!(err.field, "args[0]");
        let http = Transport::Http {
            url: "https://h/mcp".into(),
            headers: BTreeMap::from([("X-Key".to_owned(), "${NOPE}".to_owned())]),
        };
        let err = http.expanded(&lookup).unwrap_err();
        assert_eq!(err.field, "headers.X-Key");
        assert!(err.message.contains("NOPE"));
    }

    #[test]
    fn expanded_values_lists_what_variables_expanded_to() {
        let lookup = env(&[("TOKEN", "t1"), ("EMPTY", "")]);
        let http = Transport::Http {
            url: "https://h/mcp?key=${TOKEN}".into(),
            headers: BTreeMap::from([("X-E".to_owned(), "${EMPTY:-}".to_owned())]),
        };
        let (_, values) = http.expanded_values(&lookup).unwrap();
        assert_eq!(
            values,
            vec!["t1".to_owned()],
            "empty values are not secrets to hide"
        );
    }

    #[test]
    fn an_url_that_is_not_http_after_expansion_is_an_error() {
        let http = Transport::Http {
            url: "${BASE}/mcp".into(),
            headers: BTreeMap::new(),
        };
        let err = http.expanded(&env(&[("BASE", "ftp://h")])).unwrap_err();
        assert_eq!(err.field, "url");
    }
}
