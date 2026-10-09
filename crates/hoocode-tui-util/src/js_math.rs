//! JavaScript number helpers shared by the TUI and the tool crates.

/// JS `Math.round`: ties round toward +infinity. Callers that need an
/// integer cast the result (`as u32`, `as i64`) after this; the cast is the
/// same floor-then-cast the callers did before.
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

#[cfg(test)]
mod tests {
    use super::js_round;

    #[test]
    fn ties_round_toward_positive_infinity() {
        assert_eq!(js_round(0.5), 1.0);
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(1.49), 1.0);
        assert_eq!(js_round(-0.5), 0.0);
        assert_eq!(js_round(-2.5), -2.0);
    }
}
