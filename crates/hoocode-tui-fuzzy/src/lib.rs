//! Fuzzy matching for the hoocode TUI.
//!
//! Matches if all query characters appear in order (not necessarily
//! consecutive). Lower score = better match.

/// Result of a fuzzy-match attempt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuzzyMatch {
    pub matches: bool,
    pub score: f64,
}

fn is_word_boundary_char(c: char) -> bool {
    c.is_whitespace() || matches!(c, '-' | '_' | '.' | '/' | ':')
}

fn match_query(normalized_query: &[char], text_lower: &[char]) -> FuzzyMatch {
    if normalized_query.is_empty() {
        return FuzzyMatch {
            matches: true,
            score: 0.0,
        };
    }
    if normalized_query.len() > text_lower.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }

    let mut query_index = 0usize;
    let mut score = 0.0f64;
    let mut last_match_index: i64 = -1;
    let mut consecutive_matches = 0i64;

    for (i, &ch) in text_lower.iter().enumerate() {
        if query_index >= normalized_query.len() {
            break;
        }
        if ch != normalized_query[query_index] {
            continue;
        }

        let is_word_boundary = i == 0 || is_word_boundary_char(text_lower[i - 1]);

        if last_match_index == i as i64 - 1 {
            consecutive_matches += 1;
            score -= consecutive_matches as f64 * 5.0;
        } else {
            consecutive_matches = 0;
            if last_match_index >= 0 {
                score += (i as i64 - last_match_index - 1) as f64 * 2.0;
            }
        }

        if is_word_boundary {
            score -= 10.0;
        }
        score += i as f64 * 0.1;

        last_match_index = i as i64;
        query_index += 1;
    }

    if query_index < normalized_query.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }

    if normalized_query == text_lower {
        score -= 100.0;
    }

    FuzzyMatch {
        matches: true,
        score,
    }
}

/// Split a lowercase query into `(digits, letters)` if it's letters-then-digits
/// or digits-then-letters, so `"v2"` can also match text ordered as `"2v"`.
fn swapped_alnum_query(query_lower: &str) -> Option<String> {
    let chars: Vec<char> = query_lower.chars().collect();
    if chars.is_empty() {
        return None;
    }

    let split = chars.iter().position(|c| !c.is_ascii_lowercase());
    match split {
        Some(idx) if idx > 0 => {
            let (letters, rest) = chars.split_at(idx);
            if rest.iter().all(|c| c.is_ascii_digit()) && !rest.is_empty() {
                let mut swapped: String = rest.iter().collect();
                swapped.extend(letters);
                return Some(swapped);
            }
            None
        }
        _ => {
            // Try digits-then-letters.
            let split = chars.iter().position(|c| !c.is_ascii_digit());
            match split {
                Some(idx) if idx > 0 => {
                    let (digits, rest) = chars.split_at(idx);
                    if rest.iter().all(|c| c.is_ascii_lowercase()) && !rest.is_empty() {
                        let mut swapped: String = rest.iter().collect();
                        swapped.extend(digits);
                        Some(swapped)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
    }
}

/// A query lowered once, with its digit/letter-swapped form. Matching one
/// query against many texts (a filter, a search) then lowers the query once.
#[derive(Debug, Clone)]
pub struct FuzzyQuery {
    query: Vec<char>,
    swapped: Option<Vec<char>>,
}

impl FuzzyQuery {
    pub fn new(query: &str) -> Self {
        let query_lower = query.to_lowercase();
        Self {
            swapped: swapped_alnum_query(&query_lower).map(|s| s.chars().collect()),
            query: query_lower.chars().collect(),
        }
    }

    /// Match against `text`, lower-cased here.
    pub fn score(&self, text: &str) -> FuzzyMatch {
        self.score_lowered(&lower_chars(text))
    }

    /// Match against text that is already lower-cased (see [`lower_chars`]).
    fn score_lowered(&self, text_lower: &[char]) -> FuzzyMatch {
        let primary = match_query(&self.query, text_lower);
        if primary.matches {
            return primary;
        }

        let Some(swapped) = &self.swapped else {
            return primary;
        };
        let swapped_match = match_query(swapped, text_lower);
        if !swapped_match.matches {
            return primary;
        }

        FuzzyMatch {
            matches: true,
            score: swapped_match.score + 5.0,
        }
    }
}

/// The lower-cased characters of `text`.
fn lower_chars(text: &str) -> Vec<char> {
    text.to_lowercase().chars().collect()
}

/// Fuzzy-match `query` against `text`. Lower score is a better match.
pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
    FuzzyQuery::new(query).score(text)
}

/// Filter and sort `items` by fuzzy-match quality against `query` (best
/// matches first). Space-separated query tokens must all match.
pub fn fuzzy_filter<T: Clone>(items: &[T], query: &str, get_text: impl Fn(&T) -> String) -> Vec<T> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return items.to_vec();
    }

    let tokens: Vec<FuzzyQuery> = trimmed.split_whitespace().map(FuzzyQuery::new).collect();
    if tokens.is_empty() {
        return items.to_vec();
    }

    let mut results: Vec<(T, f64)> = Vec::new();
    for item in items {
        let text = lower_chars(&get_text(item));
        let mut total_score = 0.0;
        let mut all_match = true;

        for token in &tokens {
            let m = token.score_lowered(&text);
            if m.matches {
                total_score += m.score;
            } else {
                all_match = false;
                break;
            }
        }

        if all_match {
            results.push((item.clone(), total_score));
        }
    }

    results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    results.into_iter().map(|(item, _)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything_with_score_zero() {
        let m = fuzzy_match("", "anything");
        assert!(m.matches);
        assert_eq!(m.score, 0.0);
    }

    #[test]
    fn query_longer_than_text_does_not_match() {
        assert!(!fuzzy_match("abcdef", "abc").matches);
        assert!(!fuzzy_match("longquery", "short").matches);
    }

    #[test]
    fn exact_match_scores_best() {
        let exact = fuzzy_match("hello", "hello");
        let partial = fuzzy_match("hello", "hello world");
        assert!(exact.matches);
        assert!(partial.matches);
        assert!(
            exact.score < partial.score,
            "exact match should score lower (better)"
        );
        let test = fuzzy_match("test", "test");
        assert!(test.matches);
        assert!(test.score < 0.0);
    }

    #[test]
    fn characters_must_appear_in_order() {
        assert!(!fuzzy_match("ba", "ab").matches);
        assert!(!fuzzy_match("abc", "cba").matches);
        assert!(fuzzy_match("abc", "aXbXc").matches);
    }

    #[test]
    fn non_consecutive_characters_still_match() {
        assert!(fuzzy_match("fb", "foobar").matches);
    }

    #[test]
    fn word_boundary_matches_score_better() {
        // "gs" starts two words in "get_stuff" (g, then s after _).
        let boundary = fuzzy_match("gs", "get_stuff");
        let no_boundary = fuzzy_match("gs", "xgxsx");
        assert!(boundary.matches && no_boundary.matches);
        assert!(boundary.score < no_boundary.score);

        let at = fuzzy_match("fb", "foo-bar");
        let not_at = fuzzy_match("fb", "afbx");
        assert!(at.matches && not_at.matches);
        assert!(at.score < not_at.score);
    }

    #[test]
    fn consecutive_matches_score_better_than_scattered() {
        let consecutive = fuzzy_match("foo", "foobar");
        let scattered = fuzzy_match("foo", "f_o_o_bar");
        assert!(consecutive.matches && scattered.matches);
        assert!(consecutive.score < scattered.score);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(fuzzy_match("HELLO", "hello world").matches);
        assert!(fuzzy_match("ABC", "abc").matches);
        assert!(fuzzy_match("abc", "ABC").matches);
    }

    #[test]
    fn swapped_alpha_and_digit_tokens_match() {
        // "2v" matches "v2" via the swap fallback.
        assert!(fuzzy_match("2v", "v2-model").matches);
        assert!(fuzzy_match("codex52", "gpt-5.2-codex").matches);
    }

    #[test]
    fn filter_with_empty_query_returns_all_items_unchanged() {
        let items = vec!["a", "b", "c"];
        assert_eq!(fuzzy_filter(&items, "", |s| s.to_string()), items);
    }

    #[test]
    fn filter_keeps_only_matching_items() {
        let items = vec!["apple", "banana", "cherry"];
        let r = fuzzy_filter(&items, "an", |s| s.to_string());
        assert!(r.contains(&"banana"));
        assert!(!r.contains(&"apple"));
        assert!(!r.contains(&"cherry"));

        let items = vec!["apple", "banana"];
        assert!(fuzzy_filter(&items, "xyz", |s| s.to_string()).is_empty());
    }

    #[test]
    fn filter_sorts_best_matches_first() {
        let items = vec!["xylophone", "hello", "help"];
        assert_eq!(
            fuzzy_filter(&items, "hel", |s| s.to_string()),
            vec!["hello", "help"]
        );
        let items = vec!["a_p_p", "app", "application"];
        assert_eq!(fuzzy_filter(&items, "app", |s| s.to_string())[0], "app");
    }

    #[test]
    fn filter_prioritizes_exact_match_over_longer_prefix_match() {
        let items = vec!["clone", "cl"];
        assert_eq!(
            fuzzy_filter(&items, "cl", |s| s.to_string()),
            vec!["cl", "clone"]
        );
    }

    #[test]
    fn filter_multi_token_requires_all_tokens() {
        let items = vec!["foo bar", "foo baz", "qux"];
        assert_eq!(
            fuzzy_filter(&items, "foo bar", |s| s.to_string()),
            vec!["foo bar"]
        );
    }

    #[test]
    fn filter_works_with_custom_get_text() {
        let items = [("foo", 1), ("bar", 2), ("foobar", 3)];
        let r = fuzzy_filter(&items, "foo", |i| i.0.to_string());
        assert_eq!(r.len(), 2);
        let names: Vec<_> = r.iter().map(|i| i.0).collect();
        assert!(names.contains(&"foo") && names.contains(&"foobar"));
    }

    #[test]
    fn fuzzy_query_scores_like_fuzzy_match() {
        for (query, text) in [
            ("hello", "hello world"),
            ("2V", "v2-model"),
            ("GS", "get_stuff"),
            ("zz", "abc"),
            ("", "anything"),
        ] {
            assert_eq!(
                FuzzyQuery::new(query).score(text),
                fuzzy_match(query, text),
                "{query:?} against {text:?}"
            );
        }
    }
}
