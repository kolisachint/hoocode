//! The `!` bash command: running it, its rows, and the pending rows of a streaming turn.

use hoocode_code_tool_bash::BashResult;
use hoocode_code_tui_widgets::bash_execution::BashExecutionComponent;

use super::*;

impl Mode {
    /// Out of bash mode after the prompt was cleared. The pin's `setText("")`
    /// already ran `onChange` synchronously; here that change is still queued
    /// and would see no transition, so the prefix is reset too.
    pub(super) fn leave_bash_mode(&mut self) {
        self.is_bash_mode = false;
        self.update_editor_border_color();
        self.update_editor_prompt_prefix();
    }

    /// `BashExecutionController.handleBashCommand`: run a `!` command
    /// through the session off the UI thread, streaming into its row. While
    /// the agent streams the row waits in the pending area. The `user_bash`
    /// extension hook waits on the extension runner (12.3).
    pub(super) fn handle_bash_command(&mut self, command: String, exclude_from_context: bool) {
        let component = handle(BashExecutionComponent::new(&command, exclude_from_context));
        if self.session.is_streaming() {
            self.pending_messages
                .borrow_mut()
                .add_child(as_component(&component));
            self.pending_bash_components.push(component.clone());
        } else {
            self.add_to_chat(as_component(&component));
        }
        self.bash_component = Some(component);
        self.dirty.set(true);

        let session = self.session.clone();
        let tx = self.tx.clone();
        hoocode_runtime::spawn_thread("hoocode-ui-task", move || {
            let chunks = tx.clone();
            let mut on_chunk = move |chunk: &str| {
                let _ = chunks.send(AppEvent::BashChunk(chunk.to_string()));
            };
            let result = session
                .execute_bash(&command, Some(&mut on_chunk), exclude_from_context, None)
                .map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::BashDone(result));
        });
    }

    /// The `!` command ended: settle its row.
    pub(super) fn finish_bash_command(&mut self, result: Result<BashResult, String>) {
        let component = self.bash_component.take();
        match result {
            Ok(result) => {
                if let Some(component) = component {
                    component.borrow_mut().set_complete(
                        result.exit_code,
                        result.cancelled,
                        result.truncated.then(|| truncated_marker(&result.output)),
                        result.full_output_path,
                    );
                }
            }
            Err(error) => {
                if let Some(component) = component {
                    component.borrow_mut().set_complete(None, false, None, None);
                }
                self.show_error(&format!("Bash command failed: {error}"));
            }
        }
        self.dirty.set(true);
    }

    /// `flushPendingBashComponents`: move parked `!` rows into the chat.
    pub(super) fn flush_pending_bash_components(&mut self) {
        for component in std::mem::take(&mut self.pending_bash_components) {
            let component = as_component(&component);
            self.pending_messages.borrow_mut().remove_child(&component);
            self.add_to_chat(component);
        }
    }
}
