//! The `/settings` pane and the changes it applies.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use hoocode_ai_types::Transport;
use hoocode_code_agent_session::ToolSource;
use hoocode_code_models::parse_thinking_level;
use hoocode_code_settings::platform_targets::{get_workspace_platforms, set_platforms};
use hoocode_code_settings::EditorBorder;
use hoocode_code_tools::light::{measure_prompt_surface, measure_tool_schema_tokens};
use hoocode_code_tui_selectors::settings_selector::{
    SettingsChange, SettingsConfig, SettingsSelectorComponent, ToolGroupInfo, ToolToggleInfo,
};
use hoocode_code_tui_theme::get_available_themes;
use hoocode_code_tui_theme::set_theme;
use hoocode_tui_components::FrameBorderStyle;
use hoocode_tui_images::get_capabilities;
use hoocode_tui_render::Component;

use crate::input_frame::set_input_frame_border;

use super::*;

impl Mode {
    /// `showSettingsSelector`: the `/settings` pane in the editor's slot,
    /// built from the live session.
    pub(super) fn show_settings_selector(&mut self) {
        let session = self.session.clone();
        let config = {
            let settings = self.session.settings();
            let disabled: HashSet<String> = settings.disabled_tools().into_iter().collect();
            let all_tools = self.session.get_all_tools();
            // Union so tools disabled at startup (absent from the live
            // registry) still appear and can be re-enabled for next session.
            let names: Vec<String> = all_tools
                .iter()
                .filter(|t| t.source == ToolSource::Builtin)
                .map(|t| t.name.clone())
                .chain(disabled.iter().cloned())
                .collect::<HashSet<_>>()
                .into_iter()
                .collect();
            // Order is the pane's: it sorts its rows by hoocode-ts's tool names.
            let tokens: HashMap<&str, usize> = all_tools
                .iter()
                .map(|t| {
                    (
                        t.name.as_str(),
                        measure_tool_schema_tokens(&t.name, &t.description, &t.parameters),
                    )
                })
                .collect();
            let tools = names
                .into_iter()
                .map(|name| ToolToggleInfo {
                    enabled: !disabled.contains(&name),
                    tokens: tokens.get(name.as_str()).copied(),
                    name,
                })
                .collect();
            let learn = settings.learn_settings();
            SettingsConfig {
                auto_compact: settings.compaction_enabled(),
                tools,
                tool_groups: vec![
                    ToolGroupInfo {
                        id: "web".into(),
                        label: "Web tools".into(),
                        description: "WebFetch + WebSearch (network access).".into(),
                        enabled: settings.enable_web_tools(),
                    },
                    ToolGroupInfo {
                        id: "embsearch".into(),
                        label: "Semantic search".into(),
                        description: "Semantic index layer fused into the always-on search tool."
                            .into(),
                        enabled: settings.enable_semantic_index(),
                    },
                ],
                // Resolved fresh on every open, never downloaded.
                // Extension flags arrive with the extension runner (12.3).
                flags: Vec::new(),
                tool_output_view: self.tool_output_view,
                tool_output_max_bytes: settings.tool_output_max_bytes(),
                tool_output_max_lines: settings.tool_output_max_lines(),
                context_gc: settings.context_gc_enabled(),
                show_images: settings.show_images(),
                image_width_cells: settings.image_width_cells(),
                auto_resize_images: settings.image_auto_resize(),
                block_images: settings.block_images(),
                enable_skill_commands: settings.enable_skill_commands(),
                light: settings.light(),
                plugin_install_scope: settings.plugin_install_scope(),
                enable_plugin_tools: settings.enable_plugin_tools(),
                project_pinned_settings: settings.project_settings().keys().cloned().collect(),
                platform: get_workspace_platforms().unwrap_or_default(),
                steering_mode: self.session.steering_mode(),
                follow_up_mode: self.session.follow_up_mode(),
                transport: transport_name(settings.transport()),
                thinking_level: self.session.thinking_level().as_str().to_string(),
                available_thinking_levels: self
                    .session
                    .get_available_thinking_levels()
                    .iter()
                    .map(|l| l.as_str().to_string())
                    .collect(),
                current_theme: settings.theme().unwrap_or_else(|| "dark".into()),
                available_themes: get_available_themes(),
                hide_thinking_block: self.hide_thinking_block,
                collapse_changelog: settings.collapse_changelog(),
                enable_install_telemetry: settings.enable_install_telemetry(),
                double_escape_action: settings.double_escape_action(),
                tree_filter_mode: settings.tree_filter_mode(),
                show_hardware_cursor: settings.show_hardware_cursor(),
                editor_border: settings.editor_border(),
                editor_padding_x: settings.editor_padding_x(),
                autocomplete_max_visible: settings.autocomplete_max_visible(),
                quiet_startup: settings.quiet_startup(),
                tips_enabled: settings.tips_enabled(),
                clear_on_shrink: settings.clear_on_shrink(),
                show_terminal_progress: settings.show_terminal_progress(),
                warnings: settings.warnings(),
                voice_silence_ms: resolve_voice_silence_ms(settings.voice_silence_ms()),
                webtools_timeout_secs: settings.webtools_timeout_secs(),
                learn,
                // Re-measured on every change so the pane can price a toggle.
                measure_token_surface: Some(Rc::new(move || {
                    let tools = session.agent().with_state(|s| s.tools.clone());
                    measure_prompt_surface(&session.system_prompt(), &tools.0)
                })),
                supports_images: get_capabilities().images.is_some(),
                scoped_models_summary: match settings.scoped_models().map_or(0, |m| m.len()) {
                    0 => "all models".into(),
                    1 => "1 model".into(),
                    n => format!("{n} models"),
                },
            }
        };
        self.settings_changes.borrow_mut().clear();
        let sink = self.settings_changes.clone();
        let selector = handle(SettingsSelectorComponent::new(config, move |change| {
            sink.borrow_mut().push(change)
        }));
        let list = selector.borrow().settings_list();
        {
            let mut container = self.editor_container.borrow_mut();
            container.clear();
            container.add_child(as_component(&selector));
        }
        self.tui.set_focus(Some(list));
        self.settings_selector = Some(selector);
        self.dirty.set(true);
    }

    /// Apply what the settings pane asked for.
    pub(super) fn poll_settings_selector(&mut self) {
        if self.settings_selector.is_none() {
            return;
        }
        let changes = std::mem::take(&mut *self.settings_changes.borrow_mut());
        if changes.is_empty() {
            return;
        }
        for change in changes {
            self.apply_settings_change(change);
        }
        // The pane priced the change before it was applied.
        if let Some(selector) = &self.settings_selector {
            selector.borrow().refresh_token_surface();
        }
    }

    /// `SettingsCallbacks`, one change at a time.
    fn apply_settings_change(&mut self, change: SettingsChange) {
        self.dirty.set(true);
        match change {
            SettingsChange::Cancel => {
                self.settings_selector = None;
                self.restore_editor();
            }
            SettingsChange::AutoCompact(enabled) => {
                self.session.set_auto_compaction_enabled(enabled);
                self.footer.borrow_mut().set_auto_compact_enabled(enabled);
            }
            SettingsChange::ToolEnabled { name, enabled } => {
                // Persisted for future sessions (the startup denylist).
                let mut disabled: Vec<String> = self.session.settings().disabled_tools();
                disabled.retain(|n| *n != name);
                if !enabled {
                    disabled.push(name.clone());
                }
                self.session.settings().set_disabled_tools(&disabled);
                // Live for this session; a tool removed before launch comes
                // back next session.
                let mut active = self.session.get_active_tool_names();
                active.retain(|n| *n != name);
                if enabled && self.session.get_tool_definition(&name).is_some() {
                    active.push(name);
                }
                self.session.set_active_tools_by_name(&active);
                self.footer_data.set_subagent_enabled(
                    self.session
                        .get_active_tool_names()
                        .iter()
                        .any(|t| t == "Agent"),
                );
            }
            SettingsChange::ToolGroup { id, enabled } => {
                // These gate tool creation: next session.
                let mut settings = self.session.settings();
                match id.as_str() {
                    "web" => settings.set_enable_web_tools(enabled),
                    "embsearch" => settings.set_enable_semantic_index(enabled),
                    _ => {}
                }
            }
            SettingsChange::ToolOutputView(view) => self.apply_tool_output_view(view, true),
            SettingsChange::ToolOutputMaxBytes(bytes) => self
                .session
                .settings()
                .set_tool_output_max_bytes(bytes as i64),
            SettingsChange::ToolOutputMaxLines(lines) => self
                .session
                .settings()
                .set_tool_output_max_lines(lines as i64),
            SettingsChange::ContextGc(enabled) => {
                self.session.settings().set_context_gc_enabled(enabled)
            }
            SettingsChange::Flag { name, value } => {
                // Applied live with the extension runner (12.3).
                self.session.settings().set_flag_override(&name, value);
            }
            SettingsChange::ShowImages(enabled) => {
                self.session.settings().set_show_images(enabled);
                for block in self.tool_blocks() {
                    block.borrow_mut().set_show_images(enabled);
                }
            }
            SettingsChange::ImageWidthCells(width) => {
                self.session.settings().set_image_width_cells(width as i64);
                for block in self.tool_blocks() {
                    block.borrow_mut().set_image_width_cells(width as u32);
                }
            }
            SettingsChange::AutoResizeImages(enabled) => {
                self.session.settings().set_image_auto_resize(enabled)
            }
            SettingsChange::BlockImages(blocked) => {
                self.session.settings().set_block_images(blocked)
            }
            SettingsChange::EnableSkillCommands(enabled) => {
                self.session.settings().set_enable_skill_commands(enabled);
                self.setup_autocomplete_provider();
            }
            SettingsChange::Light(enabled) => self.session.settings().set_light(enabled),
            SettingsChange::PluginInstallScope(scope) => {
                self.session.settings().set_plugin_install_scope(scope)
            }
            SettingsChange::EnablePluginTools(enabled) => {
                self.session.settings().set_enable_plugin_tools(enabled)
            }
            SettingsChange::Platform(platforms) => {
                let tokens: Vec<String> =
                    platforms.iter().map(|p| p.as_str().to_string()).collect();
                self.session.settings().set_platform(Some(&tokens));
                set_platforms(Some(&platforms));
            }
            SettingsChange::SteeringMode(mode) => self.session.set_steering_mode(mode),
            SettingsChange::FollowUpMode(mode) => self.session.set_follow_up_mode(mode),
            SettingsChange::Transport(name) => {
                if let Ok(transport) =
                    serde_json::from_value::<Transport>(serde_json::Value::String(name))
                {
                    self.session.settings().set_transport(transport);
                    self.session.agent().set_transport(Some(transport));
                }
            }
            SettingsChange::ThinkingLevel(level) => {
                if let Some(level) = parse_thinking_level(&level) {
                    self.session.set_thinking_level(level);
                    self.footer.borrow_mut().invalidate();
                    self.update_editor_border_color();
                }
            }
            SettingsChange::Theme(name) => {
                let result = set_theme(&name, true);
                self.session.settings().set_theme(&name);
                self.tui.invalidate();
                if let Err(error) = result {
                    self.show_error(&format!(
                        "Failed to load theme \"{name}\": {error}\nFell back to dark theme."
                    ));
                }
            }
            SettingsChange::ThemePreview(name) => {
                if set_theme(&name, true).is_ok() {
                    self.tui.invalidate();
                }
            }
            SettingsChange::HideThinkingBlock(hidden) => {
                self.hide_thinking_block = hidden;
                self.session.settings().set_hide_thinking_block(hidden);
                // `rebuildChatFromMessages`: the transcript again, with the
                // new thinking display.
                self.reset_transcript_view();
                let messages = self.session.messages();
                self.render_session_context(&messages, false);
            }
            SettingsChange::CollapseChangelog(collapsed) => {
                self.session.settings().set_collapse_changelog(collapsed)
            }
            SettingsChange::EnableInstallTelemetry(enabled) => self
                .session
                .settings()
                .set_enable_install_telemetry(enabled),
            SettingsChange::QuietStartup(enabled) => {
                self.session.settings().set_quiet_startup(enabled)
            }
            SettingsChange::TipsEnabled(enabled) => {
                self.session.settings().set_tips_enabled(enabled)
            }
            SettingsChange::DoubleEscapeAction(action) => {
                self.session.settings().set_double_escape_action(action)
            }
            SettingsChange::TreeFilterMode(mode) => {
                self.session.settings().set_tree_filter_mode(mode)
            }
            SettingsChange::ShowHardwareCursor(enabled) => {
                self.session.settings().set_show_hardware_cursor(enabled);
                self.tui.set_show_hardware_cursor(enabled);
            }
            SettingsChange::EditorBorder(border) => {
                self.session.settings().set_editor_border(border);
                let style = match border {
                    EditorBorder::Box => FrameBorderStyle::Box,
                    EditorBorder::Rule => FrameBorderStyle::Rule,
                };
                set_input_frame_border(style);
                self.editor.borrow_mut().editor.set_border(style);
            }
            SettingsChange::EditorPaddingX(padding) => {
                self.session.settings().set_editor_padding_x(padding as i64);
                self.editor
                    .borrow_mut()
                    .editor
                    .set_padding_x(padding as usize);
            }
            SettingsChange::AutocompleteMaxVisible(max_visible) => {
                self.session
                    .settings()
                    .set_autocomplete_max_visible(max_visible as i64);
                self.editor
                    .borrow_mut()
                    .editor
                    .set_autocomplete_max_visible(max_visible as usize);
            }
            SettingsChange::ClearOnShrink(enabled) => {
                self.session.settings().set_clear_on_shrink(enabled);
                self.tui.set_clear_on_shrink(enabled);
            }
            SettingsChange::ShowTerminalProgress(enabled) => {
                self.session.settings().set_show_terminal_progress(enabled)
            }
            SettingsChange::Warnings(warnings) => self.session.settings().set_warnings(warnings),
            SettingsChange::VoiceSilenceMs(ms) => {
                // Voice input is not ported; the value is kept for it.
                self.session.settings().set_voice_silence_ms(ms as i64)
            }
            SettingsChange::WebtoolsTimeoutSecs(secs) => self
                .session
                .settings()
                .set_webtools_timeout_secs(secs as i64),
            SettingsChange::LearnSetting(key, value) => {
                self.session.settings().set_learn_setting(key, value as i64)
            }
            SettingsChange::OpenScopedModels => {
                // The pane gives way to the picker, as /scoped-models does.
                self.settings_selector = None;
                self.restore_editor();
                self.show_models_selector();
            }
        }
    }
}
