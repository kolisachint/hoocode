//! The TUI's plugin renderer list must name exactly the real plugin tools
//! (`hoocode-code-permissions` `PLUGIN_TOOLS`). The widgets crate copies the
//! list because it does not depend on the permissions crate; this checks the copy.

use hoocode_code_permissions::PLUGIN_TOOLS;
use hoocode_code_tui_widgets::tools::plugins::RENDERED_PLUGIN_TOOLS;

#[test]
fn rendered_plugin_tools_match_real_plugin_tools() {
    let mut rendered = RENDERED_PLUGIN_TOOLS.to_vec();
    let mut real = PLUGIN_TOOLS.to_vec();
    rendered.sort_unstable();
    real.sort_unstable();
    assert_eq!(rendered, real);
}
