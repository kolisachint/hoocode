//! The InteractiveMode half of the pin's
//! `test/interactive-mode-anthropic-warning.test.ts`: the once-per-session latch.
//! The notice itself is a persistent status-line row (`extra usage  /settings`,
//! `StatusLineData::extra_usage`), fed by the same trigger on the session
//! (`AgentSession::uses_anthropic_subscription_auth`, off switch
//! `warnings.anthropicExtraUsage`). Which auth counts is tested on the session
//! (`hoocode-code-agent-session/tests/anthropic_subscription_auth.rs`).

use std::cell::Cell;

use hoocode_code_tui_app::mode::claim_anthropic_subscription_warning;
use hoocode_code_tui_app::status_line::{
    status_line_text, LeftSlot, NoticeLevel, StatusLineData, StatusNotice,
};
use hoocode_tui_ratatui::Palette;

use crate::support::lock;

#[test]
fn warns_once_when_anthropic_subscription_auth_is_detected() {
    let mut shown = false;
    let lookups = Cell::new(0);
    let uses = || {
        lookups.set(lookups.get() + 1);
        true
    };
    let first = claim_anthropic_subscription_warning(&mut shown, uses);
    let second = claim_anthropic_subscription_warning(&mut shown, uses);
    assert!(first);
    assert!(shown);
    assert!(!second);
    assert_eq!(lookups.get(), 1);
}

#[test]
fn leaves_the_latch_unclaimed_for_non_subscription_auth() {
    let mut shown = false;
    assert!(!claim_anthropic_subscription_warning(&mut shown, || false));
    assert!(!shown);
}

fn text(data: &StatusLineData) -> String {
    let _g = lock(Some("dark"));
    status_line_text(data, 100, &Palette::current())
}

#[test]
fn extra_usage_is_a_dim_row_with_the_settings_off_switch() {
    let data = StatusLineData {
        extra_usage: true,
        ..StatusLineData::default()
    };
    assert_eq!(data.left_slot(), LeftSlot::ExtraUsage);
    let row = text(&data);
    assert!(row.contains("extra usage"), "{row}");
    assert!(row.contains("/settings"), "{row}");
    assert!(!row.contains('⊗'), "{row}");
}

#[test]
fn extra_usage_yields_to_a_live_notice_and_outranks_signed_out() {
    let mut data = StatusLineData {
        extra_usage: true,
        signed_out: true,
        notice: Some(StatusNotice::new(NoticeLevel::Info, "hello")),
        ..StatusLineData::default()
    };
    assert!(matches!(data.left_slot(), LeftSlot::Notice(_)));
    data.notice = None;
    assert_eq!(data.left_slot(), LeftSlot::ExtraUsage);
    data.extra_usage = false;
    assert_eq!(data.left_slot(), LeftSlot::NotSignedIn);
}

#[test]
fn skip_permissions_sits_below_extra_usage_and_above_signed_out() {
    let mut data = StatusLineData {
        skip_permissions: true,
        signed_out: true,
        ..StatusLineData::default()
    };
    assert_eq!(data.left_slot(), LeftSlot::SkipPermissions);
    let row = text(&data);
    assert!(row.contains("skip perms · mode rules apply"), "{row}");
    data.extra_usage = true;
    assert_eq!(data.left_slot(), LeftSlot::ExtraUsage);
    data.extra_usage = false;
    data.skip_permissions = false;
    assert_eq!(data.left_slot(), LeftSlot::NotSignedIn);
}
