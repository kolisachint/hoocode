//! Core CLI tools: read, bash, edit, write, grep, find, ls.
//!
//! The default bundle: `read`, `edit`, `write` from
//! `hoocode-code-tools-fs`, `bash` from
//! `hoocode-code-tool-bash` and `CodeSearch` from
//! `hoocode-code-tool-search`. The free functions below (grep, find,
//! ls, todo) are plain helpers, not tools. Web access is the webtools
//! plugin (`ext_webtools_fetch`, `ext_webtools_search`), not this crate.

use hoocode_agent_types::{AgentTool, AgentToolResult};
use hoocode_ai_types::{Content, TextContent};
use hoocode_code_tool_api::{wrap_tool_definitions, ToolContextFactory, ToolDefinition};
use hoocode_code_tool_bash::{create_bash_tool_definition, BashToolOptions};
use hoocode_code_tool_search::{create_search_tool_definition, SearchToolOptions};
use hoocode_code_tools_fs::{
    create_edit_tool_definition, create_read_tool_definition, create_write_tool_definition,
    EditToolOptions, ReadToolOptions, WriteToolOptions,
};
use std::path::Path;

pub mod file_finder;
pub mod light;
pub mod permissions;

pub use permissions::*;

/// Build a text-only tool result.
pub fn text_result(text: impl Into<String>) -> AgentToolResult {
    AgentToolResult {
        content: vec![Content::Text(TextContent {
            text_signature: None,
            text: text.into(),
        })],
        details: serde_json::Value::Null,
        terminate: false,
    }
}

/// Build an error tool result.
pub fn error_result(text: impl Into<String>) -> AgentToolResult {
    AgentToolResult {
        content: vec![Content::Text(TextContent {
            text_signature: None,
            text: text.into(),
        })],
        details: serde_json::Value::Null,
        terminate: false,
    }
}

/// Read the contents of a file.
pub fn read_file(path: impl AsRef<Path>) -> Result<String, std::io::Error> {
    std::fs::read_to_string(path)
}

/// Write contents to a file, creating parent directories as needed.
pub fn write_file(
    path: impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> Result<(), std::io::Error> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
}

/// Apply a single exact-string replacement in a file.
pub fn edit_file(path: impl AsRef<Path>, old_text: &str, new_text: &str) -> Result<(), EditError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)?;
    if !text.contains(old_text) {
        return Err(EditError::OldTextNotFound);
    }
    let count = text.matches(old_text).count();
    if count > 1 {
        return Err(EditError::AmbiguousOldText(count));
    }
    let updated = text.replacen(old_text, new_text, 1);
    std::fs::write(path, updated)?;
    Ok(())
}

/// Error type for file edits.
#[derive(Debug)]
pub enum EditError {
    Io(std::io::Error),
    OldTextNotFound,
    AmbiguousOldText(usize),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::Io(e) => write!(f, "io error: {}", e),
            EditError::OldTextNotFound => write!(f, "old text not found"),
            EditError::AmbiguousOldText(n) => {
                write!(f, "old text matched {} times; expected 1", n)
            }
        }
    }
}

impl std::error::Error for EditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EditError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for EditError {
    fn from(e: std::io::Error) -> Self {
        EditError::Io(e)
    }
}

/// Search file contents for a regex pattern.
pub fn grep(
    pattern: &str,
    paths: &[impl AsRef<Path>],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let regex = regex_lite::Regex::new(pattern)?;
    let mut matches = Vec::new();
    for path in paths {
        let path = path.as_ref();
        if path.is_file() {
            let text = std::fs::read_to_string(path)?;
            for (i, line) in text.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(format!("{}:{}:{}", path.display(), i + 1, line));
                }
            }
        }
    }
    Ok(matches.join("\n"))
}

/// Recursively find files matching a glob pattern under a root directory.
pub fn find(
    root: impl AsRef<Path>,
    pattern: &str,
) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error + Send + Sync>> {
    let root = root.as_ref();
    let mut results = Vec::new();
    if !root.is_dir() {
        return Ok(results);
    }
    let glob = glob::Pattern::new(pattern)?;
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.is_file() {
            let relative = path.strip_prefix(root).unwrap_or(path);
            if glob.matches_path(relative) {
                results.push(path.to_path_buf());
            }
        }
    }
    Ok(results)
}

/// List directory entries.
pub fn ls(dir: impl AsRef<Path>) -> Result<Vec<std::path::PathBuf>, std::io::Error> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    entries.sort();
    Ok(entries)
}

/// Todo list storage
use std::sync::Mutex;

static TODO_LIST: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Perform a todo action.
pub fn todo_action(
    action: &str,
    task: &str,
    id: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut todos = TODO_LIST.lock().map_err(|e| format!("Lock error: {}", e))?;

    match action {
        "add" => {
            if task.is_empty() {
                return Err("Task description required".into());
            }
            todos.push(task.to_string());
            Ok(format!("Added todo #{}: {}", todos.len(), task))
        }
        "list" => {
            if todos.is_empty() {
                Ok("No todo items".to_string())
            } else {
                let list = todos
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("{}. [ ] {}", i + 1, t))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(format!("Todo list ({} items):\n{}", todos.len(), list))
            }
        }
        "done" => {
            if id == 0 || id > todos.len() {
                return Err(
                    format!("Invalid todo id: {}. Use 'list' to see available ids.", id).into(),
                );
            }
            let completed = todos.remove(id - 1);
            Ok(format!("Completed todo #{}: {}", id, completed))
        }
        _ => Err(format!("Unknown action: {}. Use 'add', 'list', or 'done'.", action).into()),
    }
}

/// Options for [`default_tools_with`].
#[derive(Clone, Default)]
pub struct DefaultToolsOptions {
    /// Options for the `read` tool (output caps, image resize, dedup).
    pub read: ReadToolOptions,
    /// Options for the `bash` tool (shell, command prefix, output caps).
    pub bash: BashToolOptions,
    /// Supplies the model and session branch to context-aware tools.
    pub ctx_factory: Option<ToolContextFactory>,
}

/// Return the default set of coding tools ready to register with an agent.
pub fn default_tools(cwd: std::path::PathBuf, permissions: PermissionPolicy) -> Vec<AgentTool> {
    default_tools_with(cwd, permissions, DefaultToolsOptions::default())
}

/// [`default_tools`] with tool options and a context factory.
pub fn default_tools_with(
    cwd: std::path::PathBuf,
    permissions: PermissionPolicy,
    options: DefaultToolsOptions,
) -> Vec<AgentTool> {
    wrap_tool_definitions(
        default_tool_definitions(cwd, permissions, options.read, options.bash),
        options.ctx_factory,
    )
}

/// The default coding bundle (`CODING_TOOL_NAMES`): Read, Shell, Edit, Write
/// and CodeSearch, in hoocode's order. AskUserQuestion and the other extension
/// tools are registered by the CLI, after these.
/// TodoWrite is an opt-in tool (10.2f).
pub fn default_tool_definitions(
    cwd: std::path::PathBuf,
    _permissions: PermissionPolicy,
    read: ReadToolOptions,
    bash: BashToolOptions,
) -> Vec<ToolDefinition> {
    vec![
        create_read_tool_definition(cwd.clone(), read),
        create_bash_tool_definition(cwd.clone(), bash),
        create_edit_tool_definition(cwd.clone(), EditToolOptions::default()),
        create_write_tool_definition(cwd.clone(), WriteToolOptions::default()),
        create_search_tool_definition(cwd, SearchToolOptions::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "hoocode-code-tools-test-{}-{}",
            name,
            std::process::id()
        ))
    }

    #[test]
    fn test_read_write_edit() {
        let dir = temp_dir("read-write-edit");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("file.txt");
        write_file(&path, "hello world").unwrap();
        assert_eq!(read_file(&path).unwrap(), "hello world");
        edit_file(&path, "world", "rust").unwrap();
        assert_eq!(read_file(&path).unwrap(), "hello rust");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_edit_ambiguous() {
        let dir = temp_dir("ambiguous");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("file.txt");
        write_file(&path, "abc abc").unwrap();
        assert!(matches!(
            edit_file(&path, "abc", "x").unwrap_err(),
            EditError::AmbiguousOldText(2)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_ls() {
        let dir = temp_dir("ls");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_file(dir.join("a.txt"), "").unwrap();
        write_file(dir.join("b.txt"), "").unwrap();
        let entries = ls(&dir).unwrap();
        assert_eq!(entries.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_grep_and_find() {
        let dir = temp_dir("grep-find");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        write_file(dir.join("foo.rs"), "fn main() {}").unwrap();
        write_file(dir.join("bar.rs"), "fn helper() {}").unwrap();

        let matches = grep("fn main", &[dir.join("foo.rs"), dir.join("bar.rs")]).unwrap();
        assert!(matches.contains("main"));

        let found = find(&dir, "*.rs").unwrap();
        assert_eq!(found.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_todo_actions() {
        // Test add
        let result = todo_action("add", "Buy groceries", 0).unwrap();
        assert!(result.contains("Added todo"));

        // Test list
        let result = todo_action("list", "", 0).unwrap();
        assert!(result.contains("Buy groceries"));

        // Test done
        let result = todo_action("done", "", 1).unwrap();
        assert!(result.contains("Completed todo"));

        // Test list after done
        let result = todo_action("list", "", 0).unwrap();
        assert!(result.contains("No todo items"));
    }

    #[test]
    fn test_todo_invalid_id() {
        let result = todo_action("done", "", 999);
        assert!(result.is_err());
    }

    #[test]
    fn test_todo_empty_task() {
        let result = todo_action("add", "", 0);
        assert!(result.is_err());
    }
}
