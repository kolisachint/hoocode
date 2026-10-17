//! Hull paint (M8 e5): `/color` cycles the session's paint.
//!
//! The paint is separate from the theme (`/theme`): none, parity (gold),
//! cyan (acc), violet (vio), lime (ok), red (err), none. The paint name is
//! the comms frame's right label, the frame title and border take the paint
//! colour, and the HUD row takes the hull tint as its background. A toast
//! shows the name on every change.
//!
//! `/color <name>` sets the paint outright. The old session-chip colour
//! names keep working, mapped to the nearest paint; the chip itself is
//! untouched (it still cycles on its own keys).

/// What `/color` rejected: the argument it did not recognize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseHullPaintError {
    Unknown(String),
}

/// One hull paint: its display name and its palette token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HullPaint {
    Parity,
    Cyan,
    Violet,
    Lime,
    Red,
}

impl HullPaint {
    /// The paint name: the comms frame's right label and the toast.
    pub fn name(self) -> &'static str {
        match self {
            HullPaint::Parity => "parity",
            HullPaint::Cyan => "cyan",
            HullPaint::Violet => "violet",
            HullPaint::Lime => "lime",
            HullPaint::Red => "red",
        }
    }

    /// The old session-chip colour name for the notice's `· <colour>` suffix
    /// (design e5 shows `parity · yellow`): the primary old slot colour that
    /// maps to this paint.
    pub fn legacy_name(self) -> &'static str {
        match self {
            HullPaint::Parity => "yellow",
            HullPaint::Cyan => "cyan",
            HullPaint::Violet => "purple",
            HullPaint::Lime => "green",
            HullPaint::Red => "red",
        }
    }

    /// The theme token painting the notice swatch: the paint's own colour.
    pub fn swatch_token(self) -> &'static str {
        match self {
            HullPaint::Parity => "warning",
            HullPaint::Cyan => "accent",
            HullPaint::Violet => "agent2",
            HullPaint::Lime => "success",
            HullPaint::Red => "error",
        }
    }

    /// The palette token the chrome takes: gold, acc, vio, ok, err.
    pub fn token(self) -> &'static str {
        match self {
            HullPaint::Parity => "gold",
            HullPaint::Cyan => "acc",
            HullPaint::Violet => "vio",
            HullPaint::Lime => "ok",
            HullPaint::Red => "err",
        }
    }

    /// The cycle step from `current`: none, parity, cyan, violet, lime,
    /// red, none.
    pub fn next(current: Option<HullPaint>) -> Option<HullPaint> {
        match current {
            None => Some(HullPaint::Parity),
            Some(HullPaint::Parity) => Some(HullPaint::Cyan),
            Some(HullPaint::Cyan) => Some(HullPaint::Violet),
            Some(HullPaint::Violet) => Some(HullPaint::Lime),
            Some(HullPaint::Lime) => Some(HullPaint::Red),
            Some(HullPaint::Red) => None,
        }
    }

    /// Parses a `/color` argument: a paint name, `none` to clear, an old
    /// session-chip colour name mapped to the nearest paint, a slot number,
    /// or a first letter. `Err` is unrecognized.
    pub fn parse(input: &str) -> Result<Option<HullPaint>, ParseHullPaintError> {
        let normalized = input.trim().to_lowercase();
        if normalized.is_empty() {
            return Err(ParseHullPaintError::Unknown(input.to_string()));
        }
        // Slot numbers take the slot's old colour, mapped below.
        if normalized.bytes().all(|b| b.is_ascii_digit()) {
            let slot: u8 = normalized
                .parse()
                .map_err(|_| ParseHullPaintError::Unknown(input.to_string()))?;
            return old_slot_paint(slot)
                .map(Some)
                .ok_or(ParseHullPaintError::Unknown(input.to_string()));
        }
        match normalized.as_str() {
            "none" | "n" => Ok(None),
            "parity" | "yellow" | "amber" | "gold" | "orange" | "o" | "y" => {
                Ok(Some(HullPaint::Parity))
            }
            "cyan" | "teal" | "aqua" | "c" | "blue" | "b" => Ok(Some(HullPaint::Cyan)),
            "violet" | "purple" | "v" | "magenta" | "pink" | "rose" | "m" => {
                Ok(Some(HullPaint::Violet))
            }
            "lime" | "green" | "g" | "l" => Ok(Some(HullPaint::Lime)),
            "red" | "r" => Ok(Some(HullPaint::Red)),
            _ => {
                // A first letter elsewhere in either vocabulary.
                first_letter(&normalized)
                    .map(Some)
                    .ok_or(ParseHullPaintError::Unknown(input.to_string()))
            }
        }
    }
}

/// The old chip slots' canonical colours, nearest paint first: 1 cyan, 2
/// purple, 3 yellow, 4 magenta, 5 green, 6 blue.
fn old_slot_paint(slot: u8) -> Option<HullPaint> {
    match slot {
        1 => Some(HullPaint::Cyan),
        2 | 4 => Some(HullPaint::Violet),
        3 => Some(HullPaint::Parity),
        5 => Some(HullPaint::Lime),
        6 => Some(HullPaint::Cyan),
        _ => None,
    }
}

/// A first letter unique across both vocabularies, paints winning ties
/// (`p` is parity, not purple; `r` is red, whose old alias pointed at
/// magenta's slot).
fn first_letter(normalized: &str) -> Option<HullPaint> {
    let mut chars = normalized.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    match first {
        'p' => Some(HullPaint::Parity),
        'c' | 'b' => Some(HullPaint::Cyan),
        'v' | 'm' => Some(HullPaint::Violet),
        'l' | 'g' => Some(HullPaint::Lime),
        'r' => Some(HullPaint::Red),
        'y' | 'o' => Some(HullPaint::Parity),
        'n' => None,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_runs_none_parity_cyan_violet_lime_red_none() {
        let mut paint = None;
        let mut names = vec!["none"];
        for _ in 0..6 {
            paint = HullPaint::next(paint);
            names.push(paint.map_or("none", HullPaint::name));
        }
        assert_eq!(
            names,
            ["none", "parity", "cyan", "violet", "lime", "red", "none"]
        );
    }

    #[test]
    fn tokens_match_the_spec_chrome() {
        assert_eq!(HullPaint::Parity.token(), "gold");
        assert_eq!(HullPaint::Cyan.token(), "acc");
        assert_eq!(HullPaint::Violet.token(), "vio");
        assert_eq!(HullPaint::Lime.token(), "ok");
        assert_eq!(HullPaint::Red.token(), "err");
    }

    #[test]
    fn notice_names_the_paint_and_its_old_colour() {
        assert_eq!(HullPaint::Parity.legacy_name(), "yellow");
        assert_eq!(HullPaint::Cyan.legacy_name(), "cyan");
        assert_eq!(HullPaint::Violet.legacy_name(), "purple");
        assert_eq!(HullPaint::Lime.legacy_name(), "green");
        assert_eq!(HullPaint::Red.legacy_name(), "red");
    }

    #[test]
    fn parses_paint_names_and_none() {
        assert_eq!(HullPaint::parse("parity"), Ok(Some(HullPaint::Parity)));
        assert_eq!(HullPaint::parse(" CYAN "), Ok(Some(HullPaint::Cyan)));
        assert_eq!(HullPaint::parse("violet"), Ok(Some(HullPaint::Violet)));
        assert_eq!(HullPaint::parse("lime"), Ok(Some(HullPaint::Lime)));
        assert_eq!(HullPaint::parse("red"), Ok(Some(HullPaint::Red)));
        assert_eq!(HullPaint::parse("none"), Ok(None));
        assert_eq!(
            HullPaint::parse(""),
            Err(ParseHullPaintError::Unknown(String::new()))
        );
        assert_eq!(
            HullPaint::parse("bogus"),
            Err(ParseHullPaintError::Unknown("bogus".to_string()))
        );
    }

    #[test]
    fn old_chip_names_map_to_the_nearest_paint() {
        assert_eq!(HullPaint::parse("yellow"), Ok(Some(HullPaint::Parity)));
        assert_eq!(HullPaint::parse("purple"), Ok(Some(HullPaint::Violet)));
        assert_eq!(HullPaint::parse("magenta"), Ok(Some(HullPaint::Violet)));
        assert_eq!(HullPaint::parse("green"), Ok(Some(HullPaint::Lime)));
        assert_eq!(HullPaint::parse("blue"), Ok(Some(HullPaint::Cyan)));
        assert_eq!(HullPaint::parse("3"), Ok(Some(HullPaint::Parity)));
        assert_eq!(HullPaint::parse("4"), Ok(Some(HullPaint::Violet)));
        assert_eq!(
            HullPaint::parse("9"),
            Err(ParseHullPaintError::Unknown("9".to_string()))
        );
    }

    #[test]
    fn first_letters_still_work_with_paints_winning_ties() {
        assert_eq!(HullPaint::parse("p"), Ok(Some(HullPaint::Parity)));
        assert_eq!(HullPaint::parse("r"), Ok(Some(HullPaint::Red)));
        assert_eq!(HullPaint::parse("v"), Ok(Some(HullPaint::Violet)));
        assert_eq!(HullPaint::parse("m"), Ok(Some(HullPaint::Violet)));
        assert_eq!(HullPaint::parse("g"), Ok(Some(HullPaint::Lime)));
    }
}
