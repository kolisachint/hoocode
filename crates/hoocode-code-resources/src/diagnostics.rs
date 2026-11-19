//! `core/diagnostics.ts`.

/// `ResourceDiagnostic.type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticType {
    Warning,
    Error,
    Collision,
}

/// `ResourceCollision`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceCollision {
    /// `"extension" | "skill" | "prompt" | "theme"`.
    pub resource_type: String,
    pub name: String,
    pub winner_path: String,
    pub loser_path: String,
    pub winner_source: Option<String>,
    pub loser_source: Option<String>,
}

/// `ResourceDiagnostic`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDiagnostic {
    pub kind: DiagnosticType,
    pub message: String,
    pub path: Option<String>,
    pub collision: Option<ResourceCollision>,
}

impl ResourceDiagnostic {
    /// A `{type: "warning", message, path}` diagnostic.
    pub fn warning(message: impl Into<String>, path: Option<&str>) -> Self {
        Self {
            kind: DiagnosticType::Warning,
            message: message.into(),
            path: path.map(str::to_string),
            collision: None,
        }
    }

    /// A diagnostic that rejects the resource it is about (`DiagnosticType::Error`).
    pub fn error(message: impl Into<String>, path: Option<&str>) -> Self {
        Self {
            kind: DiagnosticType::Error,
            ..Self::warning(message, path)
        }
    }
}
