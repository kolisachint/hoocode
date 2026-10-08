//! Code-aware tokenization and the BM25 index over [`CapabilityEntry`]s.

use std::collections::{BTreeSet, HashMap};

use crate::CapabilityEntry;

/// BM25 term-frequency saturation.
const K1: f64 = 1.2;
/// BM25 length normalization.
const B: f64 = 0.75;

/// Words that carry no meaning in a capability query. Dropped unless the query
/// is made of nothing else.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from", "how",
    "i", "if", "in", "is", "it", "me", "my", "of", "on", "or", "so", "that", "the", "this", "to",
    "what", "where", "which", "with", "you", "your",
];

/// Split text into lowercase index terms.
///
/// Words break on any non-alphanumeric character, so `snake_case`, `kebab-case`
/// and spaces all separate. Each word then breaks at camelCase boundaries
/// (`fooBar`, `HTTPServer`) and at letter/digit boundaries. A plural token also
/// emits its singular, so `subagents` yields `subagent` as well.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        for part in split_camel(word) {
            if let Some(singular) = singular(&part) {
                out.push(singular);
            }
            out.push(part);
        }
    }
    out
}

/// Break one alphanumeric word at case and digit boundaries, lowercased.
fn split_camel(word: &str) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    let mut parts = Vec::new();
    let mut start = 0;
    for i in 1..chars.len() {
        let prev = chars[i - 1];
        let cur = chars[i];
        let next = chars.get(i + 1).copied();
        // fooBar: a lowercase letter then an uppercase one.
        let camel = prev.is_lowercase() && cur.is_uppercase();
        // HTTPServer: an uppercase letter before a lowercase one starts a word.
        let acronym =
            prev.is_uppercase() && cur.is_uppercase() && next.is_some_and(char::is_lowercase);
        // utf8, v2: letters and digits split.
        let digits = (prev.is_alphabetic() && cur.is_numeric())
            || (prev.is_numeric() && cur.is_alphabetic());
        if camel || acronym || digits {
            parts.push(chars[start..i].iter().collect::<String>().to_lowercase());
            start = i;
        }
    }
    if start < chars.len() {
        parts.push(chars[start..].iter().collect::<String>().to_lowercase());
    }
    parts
}

/// The singular of a plural-looking token, or `None` when it is not one
/// (`status`, `class`, `css`, and words of three letters or fewer).
fn singular(token: &str) -> Option<String> {
    if token.chars().count() <= 3 || !token.ends_with('s') {
        return None;
    }
    if token.ends_with("ss") || token.ends_with("us") || token.ends_with("is") {
        return None;
    }
    token.strip_suffix('s').map(str::to_string)
}

/// Query terms: the tokens minus stopwords, unless only stopwords were typed.
fn query_terms(query: &str) -> BTreeSet<String> {
    let tokens = tokenize(query);
    let content: BTreeSet<String> = tokens
        .iter()
        .filter(|t| !STOPWORDS.contains(&t.as_str()))
        .cloned()
        .collect();
    if content.is_empty() {
        tokens.into_iter().collect()
    } else {
        content
    }
}

/// A BM25 index over capability entries. Built once; searches are read-only.
#[derive(Debug, Clone)]
pub struct CapabilityIndex {
    entries: Vec<CapabilityEntry>,
    /// Term -> (entry index, term frequency in that entry).
    postings: HashMap<String, Vec<(usize, u32)>>,
    /// Token count per entry.
    doc_len: Vec<f64>,
    avg_len: f64,
}

impl CapabilityIndex {
    /// Index `entries`. The name counts twice, so a name hit outweighs a
    /// description hit.
    pub fn new(entries: Vec<CapabilityEntry>) -> Self {
        let mut postings: HashMap<String, Vec<(usize, u32)>> = HashMap::new();
        let mut doc_len = Vec::with_capacity(entries.len());
        for (doc, entry) in entries.iter().enumerate() {
            let name_tokens = tokenize(&entry.name);
            let mut tf: HashMap<String, u32> = HashMap::new();
            for token in name_tokens.iter().chain(name_tokens.iter()) {
                *tf.entry(token.clone()).or_insert(0) += 1;
            }
            for token in tokenize(&entry.description) {
                *tf.entry(token).or_insert(0) += 1;
            }
            let len: u32 = tf.values().sum();
            doc_len.push(f64::from(len));
            for (term, count) in tf {
                postings.entry(term).or_default().push((doc, count));
            }
        }
        let total: f64 = doc_len.iter().sum();
        let avg_len = if entries.is_empty() {
            1.0
        } else {
            (total / entries.len() as f64).max(1.0)
        };
        Self {
            entries,
            postings,
            doc_len,
            avg_len,
        }
    }

    /// Every indexed entry, in insertion order.
    pub fn entries(&self) -> &[CapabilityEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The best `limit` entries for `query`.
    ///
    /// An entry whose name equals the query (case-insensitively) comes first,
    /// whatever its BM25 score. The rest follow by score, then by name, so the
    /// order is stable. Only entries sharing a query term are returned.
    pub fn search(&self, query: &str, limit: usize) -> Vec<&CapabilityEntry> {
        if limit == 0 || self.entries.is_empty() {
            return Vec::new();
        }
        let n = self.entries.len() as f64;
        let mut scores = vec![0.0_f64; self.entries.len()];
        for term in query_terms(query) {
            let Some(postings) = self.postings.get(&term) else {
                continue;
            };
            let df = postings.len() as f64;
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            for &(doc, count) in postings {
                let tf = f64::from(count);
                let norm = 1.0 - B + B * self.doc_len[doc] / self.avg_len;
                scores[doc] += idf * tf * (K1 + 1.0) / (tf + K1 * norm);
            }
        }

        let wanted = query.trim().to_lowercase();
        let mut ranked: Vec<(usize, bool)> = Vec::new();
        for (doc, entry) in self.entries.iter().enumerate() {
            let exact = !wanted.is_empty() && entry.name.to_lowercase() == wanted;
            if exact || scores[doc] > 0.0 {
                ranked.push((doc, exact));
            }
        }
        ranked.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| {
                    scores[b.0]
                        .partial_cmp(&scores[a.0])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| self.entries[a.0].name.cmp(&self.entries[b.0].name))
                .then_with(|| a.0.cmp(&b.0))
        });
        ranked
            .into_iter()
            .take(limit)
            .map(|(doc, _)| &self.entries[doc])
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CapabilityKind;

    fn entry(name: &str, description: &str) -> CapabilityEntry {
        CapabilityEntry {
            kind: CapabilityKind::Skill,
            name: name.into(),
            description: description.into(),
            source: "test".into(),
        }
    }

    fn terms(text: &str) -> Vec<String> {
        tokenize(text)
    }

    #[test]
    fn tokenizes_camel_case() {
        assert_eq!(terms("searchHooCode"), ["search", "hoo", "code"]);
        assert_eq!(terms("HTTPServer"), ["http", "server"]);
    }

    #[test]
    fn tokenizes_snake_kebab_and_spaces() {
        assert_eq!(
            terms("create_pull-request now"),
            ["create", "pull", "request", "now"]
        );
    }

    #[test]
    fn splits_letters_from_digits() {
        assert_eq!(terms("utf8 v2"), ["utf", "8", "v", "2"]);
    }

    #[test]
    fn lowercases_and_folds_plurals() {
        let t = terms("Subagents");
        assert!(t.contains(&"subagents".to_string()));
        assert!(t.contains(&"subagent".to_string()));
        // Words that only look plural are left alone.
        assert_eq!(terms("status"), ["status"]);
        assert_eq!(terms("css"), ["css"]);
    }

    #[test]
    fn empty_and_punctuation_only_text_yield_no_terms() {
        assert!(terms("").is_empty());
        assert!(terms("--- !!").is_empty());
    }

    #[test]
    fn ranks_the_entry_matching_the_query_terms_first() {
        let index = CapabilityIndex::new(vec![
            entry("pdf", "Read and write PDF files."),
            entry("web-review", "Review web pages for accessibility."),
        ]);
        let hits = index.search("pdf files", 5);
        assert_eq!(hits[0].name, "pdf");
        assert_eq!(hits.len(), 1, "web-review shares no query term");
    }

    #[test]
    fn name_matches_outrank_description_matches() {
        let index = CapabilityIndex::new(vec![
            entry("notes", "Keep review notes for the team."),
            entry("review", "Run a code review."),
        ]);
        assert_eq!(index.search("review", 2)[0].name, "review");
    }

    #[test]
    fn exact_name_match_ranks_first_even_with_a_weaker_score() {
        // "security" is a common word in the long description, so it scores high
        // on BM25; the exact name still has to win.
        let index = CapabilityIndex::new(vec![
            entry(
                "security-review-deep",
                "security security security security review of code for security issues",
            ),
            entry("security", "Short."),
        ]);
        let hits = index.search("security", 5);
        assert_eq!(hits[0].name, "security");
        assert_eq!(hits[1].name, "security-review-deep");
    }

    #[test]
    fn exact_match_is_case_insensitive_and_trimmed() {
        let index = CapabilityIndex::new(vec![
            entry("other", "mentions plan in the text"),
            entry("Plan", "Plan a change."),
        ]);
        assert_eq!(index.search("  plan ", 5)[0].name, "Plan");
    }

    #[test]
    fn stopword_only_query_still_searches() {
        let index = CapabilityIndex::new(vec![entry("it", "the thing")]);
        assert_eq!(index.search("what is it", 5).len(), 1);
    }

    #[test]
    fn stopwords_do_not_pull_in_unrelated_entries() {
        let index = CapabilityIndex::new(vec![
            entry("themes", "Define the theme colors."),
            entry("quickstart", "Add a file to tell it how to work."),
        ]);
        // "add" is a content word here, so the noise entry may match too; the
        // stopwords must not put it first.
        let hits = index.search("how do I add a custom theme", 5);
        assert_eq!(hits[0].name, "themes");
    }

    #[test]
    fn limit_caps_the_results_and_zero_returns_none() {
        let index = CapabilityIndex::new(vec![
            entry("a-review", "review"),
            entry("b-review", "review"),
            entry("c-review", "review"),
        ]);
        assert_eq!(index.search("review", 2).len(), 2);
        assert!(index.search("review", 0).is_empty());
    }

    #[test]
    fn no_match_returns_nothing_and_empty_index_is_safe() {
        let index = CapabilityIndex::new(vec![entry("pdf", "PDF files")]);
        assert!(index.search("kubernetes", 5).is_empty());
        let empty = CapabilityIndex::new(Vec::new());
        assert!(empty.is_empty());
        assert!(empty.search("anything", 5).is_empty());
    }

    #[test]
    fn ties_break_by_name_for_a_stable_order() {
        let index = CapabilityIndex::new(vec![entry("zeta", "tool"), entry("alpha", "tool")]);
        let names: Vec<&str> = index
            .search("tool", 5)
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, ["alpha", "zeta"]);
    }
}
