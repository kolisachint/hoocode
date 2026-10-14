//! `core/scheduler.ts`: the 5-field cron matcher (minute hour day-of-month
//! month day-of-week, local time).
//!
//! The parser follows the TypeScript one, including its `parseInt` leniency:
//! a field part that does not start with a number is skipped, not an error.
//! Invalid expressions never match.

use chrono::{DateTime, Datelike, Local, Timelike};
use std::collections::HashSet;

/// `splitFields`: `expr.trim().split(/\s+/)`.
pub fn split_fields(expr: &str) -> Vec<&str> {
    expr.split_whitespace().collect()
}

/// `isFiveFieldCron` from `extensions/core/loop.ts`.
pub fn is_five_field(expr: &str) -> bool {
    split_fields(expr).len() == 5
}

/// JavaScript `parseInt(s)` (radix 10): leading whitespace, an optional sign,
/// then the leading digits. `None` where JavaScript gives `NaN`.
fn js_parse_int(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let (negative, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let end = digits
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let value: i64 = digits.get(..end)?.parse().ok()?;
    Some(if negative { -value } else { value })
}

/// The values one cron field allows: `*` allows everything.
enum Field {
    Any,
    Values(HashSet<i64>),
}

impl Field {
    fn allows(&self, value: i64) -> bool {
        match self {
            Field::Any => true,
            Field::Values(values) => values.contains(&value),
        }
    }
}

/// `parseField`: one field over the range `min..=max`.
fn parse_field(field: &str, min: i64, max: i64) -> Field {
    if field == "*" {
        return Field::Any;
    }
    let mut allowed = HashSet::new();
    for part in field.split(',') {
        let mut pieces = part.split('/');
        let range = pieces.next().unwrap_or("");
        let step = match pieces.next() {
            Some(step) if !step.is_empty() => match js_parse_int(step) {
                Some(step) => step,
                None => continue,
            },
            _ => 1,
        };
        if step < 1 {
            continue;
        }
        let (lo, hi) = if !range.is_empty() && range != "*" {
            let mut bounds = range.split('-');
            let a = bounds.next().unwrap_or("");
            let lo = match js_parse_int(a) {
                Some(lo) => lo,
                None => continue,
            };
            let hi = match bounds.next() {
                Some(b) => match js_parse_int(b) {
                    Some(hi) => hi,
                    None => continue,
                },
                None => lo,
            };
            (lo, hi)
        } else {
            (min, max)
        };
        // Walk lo, lo+step, ... up to hi, keeping the values inside min..=max.
        // Saturating: a huge step or bound from a stored expression must not
        // overflow (a panic in debug builds).
        let mut v = lo;
        if v < min {
            // Jump to the first value of the progression at or above min.
            let skipped = min.saturating_sub(v).saturating_add(step - 1) / step;
            v = v.saturating_add(skipped.saturating_mul(step));
        }
        while v <= hi && v <= max {
            if v >= min {
                allowed.insert(v);
            }
            v = v.saturating_add(step);
        }
    }
    Field::Values(allowed)
}

/// Whether `expr` is a 5-field expression this matcher reads as written: each
/// comma-separated part is `*`, `N` or `N-M`, optionally with `/S` (S of at
/// least 1), and every value lies in its field's range. Returns the reason
/// otherwise. `/loop` refuses an expression this rejects; the tools keep the
/// lenient matcher.
pub fn validate(expr: &str) -> Result<(), String> {
    let fields = split_fields(expr);
    let [minute, hour, dom, month, dow] = fields[..] else {
        return Err(format!(
            "A cron expression has 5 fields (minute hour day-of-month month day-of-week): \"{expr}\""
        ));
    };
    let checks = [
        ("minute", minute, 0, 59),
        ("hour", hour, 0, 23),
        ("day-of-month", dom, 1, 31),
        ("month", month, 1, 12),
        ("day-of-week", dow, 0, 7),
    ];
    for (name, field, min, max) in checks {
        check_field(field, min, max)
            .map_err(|why| format!("The {name} field \"{field}\" is not valid: {why}."))?;
    }
    Ok(())
}

/// One field's parts, each checked as [`validate`] describes.
fn check_field(field: &str, min: i64, max: i64) -> Result<(), String> {
    for part in field.split(',') {
        let range = match part.split_once('/') {
            Some((range, step)) => {
                if !step.parse::<i64>().is_ok_and(|s| s >= 1) {
                    return Err(format!("\"{part}\" needs a step of at least 1"));
                }
                range
            }
            None => part,
        };
        if range == "*" {
            continue;
        }
        let (lo, hi) = range.split_once('-').unwrap_or((range, range));
        let (Ok(lo), Ok(hi)) = (lo.parse::<i64>(), hi.parse::<i64>()) else {
            return Err(format!("\"{part}\" is not a number, a range or *"));
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!("\"{part}\" must stay within {min}-{max}"));
        }
    }
    Ok(())
}

/// `matchesCron`: true when `now` matches the 5-field expression.
pub fn matches(expr: &str, now: &DateTime<Local>) -> bool {
    let fields = split_fields(expr);
    let [m, h, dom, mon, dow] = fields[..] else {
        return false;
    };

    let minute = parse_field(m, 0, 59).allows(i64::from(now.minute()));
    let hour = parse_field(h, 0, 23).allows(i64::from(now.hour()));
    let month = parse_field(mon, 1, 12).allows(i64::from(now.month()));
    let dow_value = i64::from(now.weekday().num_days_from_sunday());
    let dom_field = parse_field(dom, 1, 31);
    // Cron allows 0 or 7 for Sunday. The field is read over 0..=7 so a range
    // such as `1-7` keeps its end; Sunday (0) matches when either is listed.
    let dow_field = parse_field(dow, 0, 7);

    if !(minute && hour && month) {
        return false;
    }

    // Standard cron semantics: when both DOM and DOW are restricted, match either.
    let dom_restricted = dom != "*";
    let dow_restricted = dow != "*";
    let day_of_month = dom_field.allows(i64::from(now.day()));
    let day_of_week = dow_field.allows(dow_value) || (dow_value == 0 && dow_field.allows(7));
    if dom_restricted && dow_restricted {
        day_of_month || day_of_week
    } else {
        day_of_month && day_of_week
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// 2026-10-08 is a Thursday (day-of-week 4).
    fn at(hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, 8, hour, minute, 30)
            .single()
            .expect("local time exists")
    }

    #[test]
    fn five_field_check_splits_on_whitespace() {
        assert!(is_five_field("  */5 * * * *  "));
        assert!(!is_five_field("*/5 * * *"));
        assert!(!is_five_field(""));
    }

    #[test]
    fn steps_lists_and_ranges_match() {
        assert!(matches("*/5 * * * *", &at(9, 15)));
        assert!(!matches("*/5 * * * *", &at(9, 16)));
        assert!(matches("0,30 9-10 * * *", &at(10, 30)));
        assert!(!matches("0,30 9-10 * * *", &at(11, 30)));
        assert!(matches("0 9 * * *", &at(9, 0)));
        assert!(matches("0-59/20 * * * *", &at(3, 40)));
        assert!(!matches("0-59/20 * * * *", &at(3, 41)));
    }

    #[test]
    fn day_of_week_accepts_seven_as_sunday() {
        // 2026-10-11 is a Sunday.
        let sunday = Local
            .with_ymd_and_hms(2026, 10, 11, 12, 0, 0)
            .single()
            .unwrap();
        assert!(matches("0 12 * * 0", &sunday));
        assert!(matches("0 12 * * 7", &sunday));
        assert!(!matches("0 12 * * 1-5", &sunday));
    }

    #[test]
    fn restricted_day_of_month_and_week_match_either() {
        // Thursday the 8th: the day-of-month matches, the weekday (Monday) does not.
        assert!(matches("30 9 8 * 1", &at(9, 30)));
        assert!(matches("30 9 1 * 4", &at(9, 30)));
        assert!(!matches("30 9 1 * 1", &at(9, 30)));
    }

    #[test]
    fn invalid_expressions_never_match() {
        assert!(!matches("* * * *", &at(9, 0)));
        assert!(!matches("x * * * *", &at(9, 0)));
        assert!(!matches("*/0 * * * *", &at(9, 0)));
    }

    #[test]
    fn a_range_ending_in_seven_covers_sunday_too() {
        // 2026-10-12 is a Monday, 2026-10-11 a Sunday.
        let monday = Local
            .with_ymd_and_hms(2026, 10, 12, 9, 0, 0)
            .single()
            .unwrap();
        let sunday = Local
            .with_ymd_and_hms(2026, 10, 11, 9, 0, 0)
            .single()
            .unwrap();
        assert!(matches("0 9 * * 1-7", &monday));
        assert!(matches("0 9 * * 1-7", &sunday));
        assert!(matches("0 9 * * 5,7", &sunday));
        assert!(!matches("0 9 * * 1-6", &sunday));
    }

    #[test]
    fn huge_steps_and_bounds_do_not_overflow() {
        // Stored by hand or by a tool; must match without panicking.
        assert!(matches("1/9223372036854775807 * * * *", &at(9, 1)));
        assert!(!matches("1/9223372036854775807 * * * *", &at(9, 2)));
        assert!(!matches("-9223372036854775807/2 * * * *", &at(9, 0)));
        assert!(!matches("9223372036854775807 * * * *", &at(9, 0)));
    }

    #[test]
    fn validate_accepts_what_the_matcher_reads_and_refuses_the_rest() {
        for ok in [
            "*/10 * * * *",
            "0 9 * * 1-5",
            "0 12 * * 7",
            "0,30 9-17/2 1,15 * *",
            "0 0 31 12 0-7",
        ] {
            assert_eq!(validate(ok), Ok(()), "{ok}");
        }
        for bad in [
            "60 * * * *",
            "*/0 * * * *",
            "x * * * *",
            "0 0 0 * *",
            "0 0 * 13 *",
            "0 0 * * 8",
            "5-3 * * * *",
            "1/ * * * *",
            "1-x * * * *",
            "*/5 * * * * *",
        ] {
            assert!(validate(bad).is_err(), "{bad} should be refused");
        }
        assert!(validate("60 * * * *").unwrap_err().contains("minute"));
    }

    #[test]
    fn js_parse_int_reads_leading_digits_only() {
        assert_eq!(js_parse_int("12abc"), Some(12));
        assert_eq!(js_parse_int(" -3"), Some(-3));
        assert_eq!(js_parse_int("abc"), None);
        assert_eq!(js_parse_int(""), None);
    }
}
