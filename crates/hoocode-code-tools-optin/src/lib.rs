//! Opt-in tools: TodoWrite (`core/tools/todo.ts`) and ask_options
//! (`extensions/core/ask-options.ts`), hoocode v0.5.89.

pub mod ask_options;
pub mod plugins;
pub mod todo;

pub use ask_options::{
    ask_options_parameters_schema, create_ask_options_tool_definition, AskOption, AskOptionsHost,
    AskQuestion, NoUi,
};
pub use plugins::{
    create_plugin_tool_definitions, PluginToolHost, INSTALL_PLUGIN_TOOL_NAME,
    LIST_PLUGINS_TOOL_NAME, PLUGIN_TOOL_NAMES, PROPOSE_PLUGIN_TOOL_NAME, SEARCH_PLUGINS_TOOL_NAME,
    UNINSTALL_PLUGIN_TOOL_NAME,
};
pub use todo::{
    create_todo_write_tool_definition, settle_dangling_main_tasks, todo_write_parameters_schema,
    StoreRef, TodoItem, TodoStatus, TODO_WRITE_TOOL_NAME,
};
