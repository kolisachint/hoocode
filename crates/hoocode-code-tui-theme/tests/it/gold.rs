//! The agent and session palette mapping, against `fixtures/theme-gold.json`.

use hoocode_code_tui_theme::*;
use serde_json::Value;

fn gold() -> Value {
    serde_json::from_str(include_str!("../fixtures/theme-gold.json")).unwrap()
}

#[test]
fn agent_and_session_palette_mapping_is_stable() {
    let gold = gold();
    for (agent, token) in gold["agents"].as_object().unwrap() {
        assert_eq!(agent_color_for(agent), token.as_str().unwrap(), "{agent}");
    }
    for (slot, token) in gold["sessions"].as_object().unwrap() {
        let Ok(slot) = slot.parse::<i64>() else {
            continue;
        };
        assert_eq!(session_color_token(slot), token.as_str().unwrap(), "{slot}");
    }
}
