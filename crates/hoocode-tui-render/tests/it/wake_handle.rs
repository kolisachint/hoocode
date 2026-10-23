//! `WakeHandle`: one `Wake` in the channel per burst of wakes.

use crate::support::*;
use hoocode_tui_render::{Tui, TuiEvent};
use std::time::Duration;

fn wakes(rx: &std::sync::mpsc::Receiver<TuiEvent>) -> usize {
    let mut n = 0;
    while let Ok(event) = rx.recv_timeout(Duration::from_millis(50)) {
        if matches!(event, TuiEvent::Wake) {
            n += 1;
        }
    }
    n
}

#[test]
fn a_burst_of_wakes_sends_one_event_until_the_loop_clears() {
    let (terminal, _term) = virtual_terminal(80, 24);
    let mut tui = Tui::new(terminal, None);
    let rx = tui.start();
    let wake = tui.wake_handle();
    for _ in 0..1000 {
        wake.wake();
    }
    assert_eq!(wakes(&rx), 1);
    // The loop takes the event; the next wake sends a new one.
    tui.accept_event(TuiEvent::Wake);
    wake.wake();
    wake.wake();
    assert_eq!(wakes(&rx), 1);
}

#[test]
fn a_wake_before_start_is_not_left_pending() {
    let (terminal, _term) = virtual_terminal(80, 24);
    let mut tui = Tui::new(terminal, None);
    let wake = tui.wake_handle();
    wake.wake();
    let rx = tui.start();
    wake.wake();
    assert_eq!(wakes(&rx), 1);
}
