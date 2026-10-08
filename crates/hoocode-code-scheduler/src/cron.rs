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
        let mut v = lo;
        if v < min {
            // Jump to the first value of the progression at or above min.
            let skipped = (min - v + step - 1) / step;
            v += skipped * step;
        }
        while v <= hi && v <= max {
            if v >= min {
                allowed.insert(v);
            }
            v += step;
        }
    }
    Field::Values(allowed)
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
    // Cron allows 0 or 7 for Sunday; the TS code maps every "7" to "0".
    let dow_value = i64::from(now.weekday().num_days_from_sunday());
    let dom_field = parse_field(dom, 1, 31);
    let dow_field = parse_field(&dow.replace('7', "0"), 0, 6);

    if !(minute && hour && month) {
        return false;
    }

    // Standard cron semantics: when both DOM and DOW are restricted, match either.
    let dom_restricted = dom != "*";
    let dow_restricted = dow != "*";
    let day_of_month = dom_field.allows(i64::from(now.day()));
    let day_of_week = dow_field.allows(dow_value);
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
    fn js_parse_int_reads_leading_digits_only() {
        assert_eq!(js_parse_int("12abc"), Some(12));
        assert_eq!(js_parse_int(" -3"), Some(-3));
        assert_eq!(js_parse_int("abc"), None);
        assert_eq!(js_parse_int(""), None);
    }
}
