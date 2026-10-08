//! Port of the pin's `test/tool-chain-summary.test.ts`.

use hoocode_code_tui_widgets::tool_chain_summary::{
    chain_phrase, chain_segments, chain_stats, ChainEntry, ChainState, SegmentTone,
};

fn call(tool: &str, subject: &str, lines: usize, is_error: bool, is_partial: bool) -> ChainEntry {
    ChainEntry {
        tool: tool.into(),
        subject: subject.into(),
        output_lines: lines,
        is_error,
        is_partial,
    }
}

fn c(tool: &str, subject: &str) -> ChainEntry {
    call(tool, subject, 0, false, false)
}

fn chain(entries: &[ChainEntry]) -> String {
    chain_segments(entries)
        .iter()
        .map(|s| s.label.clone())
        .collect::<Vec<_>>()
        .join(" › ")
}

mod chain_segments_the_running_line {
    use super::*;

    #[test]
    fn keeps_the_order_of_the_run() {
        assert_eq!(
            chain(&[c("CodeSearch", "x"), c("Read", "a.ts"), c("Edit", "a.ts")]),
            "CodeSearch › Read › Edit"
        );
    }

    #[test]
    fn collapses_a_consecutive_repeat() {
        assert_eq!(
            chain(&[
                c("CodeSearch", "x"),
                c("Read", "a"),
                c("Read", "b"),
                c("Read", "c")
            ]),
            "CodeSearch › Read ×3"
        );
    }

    #[test]
    fn never_merges_a_failure_into_a_repeat() {
        assert_eq!(
            chain(&[
                c("Read", "a"),
                call("Read", "b", 0, true, false),
                c("Read", "c")
            ]),
            "Read › Read✗ › Read"
        );
    }

    #[test]
    fn elides_the_middle_of_a_long_chain_but_never_a_failure() {
        let tools = ["Shell", "CodeSearch", "Read", "Edit"];
        let entries: Vec<ChainEntry> = (0..27)
            .map(|i| {
                if i == 9 {
                    call("Shell", "check", 0, true, false)
                } else {
                    c(tools[i % 4], &format!("f{i}"))
                }
            })
            .collect();
        let rendered = chain(&entries);
        assert!(rendered.contains("Shell✗"));
        assert!(rendered.contains("more …"));
        assert!(rendered.split(" › ").count() < entries.len());
    }

    #[test]
    fn marks_a_still_running_call_rather_than_calling_it_done() {
        let segments = chain_segments(&[call("Shell", "x", 0, false, true)]);
        assert_eq!(segments[0].tone, SegmentTone::Running);
    }
}

mod chain_stats_ {
    use super::*;

    #[test]
    fn counts_progress_while_running_totals_once_done() {
        let entries = [
            call("CodeSearch", "x", 5, false, false),
            call("Shell", "y", 0, false, true),
        ];
        assert_eq!(
            chain_stats(&entries, ChainState::Running),
            "1 done · running"
        );
        assert_eq!(
            chain_stats(
                &[
                    call("CodeSearch", "x", 5, false, false),
                    call("Shell", "y", 7, false, false)
                ],
                ChainState::Done
            ),
            "2 calls · 12 lines"
        );
    }

    #[test]
    fn always_surfaces_failures() {
        assert!(
            chain_stats(&[call("Shell", "x", 1, true, false)], ChainState::Done)
                .contains("1 failed")
        );
    }

    #[test]
    fn says_an_interrupted_chain_was_interrupted() {
        assert!(chain_stats(
            &[call("CodeSearch", "x", 1, false, false)],
            ChainState::Interrupted
        )
        .contains("interrupted"));
    }
}

mod chain_phrase_the_settled_line {
    use super::*;

    #[test]
    fn names_the_most_consequential_thing_not_the_most_frequent() {
        let mut entries: Vec<ChainEntry> = (0..6)
            .map(|i| call("Read", &format!("src/f{i}.ts"), 10, false, false))
            .collect();
        entries.push(c("Edit", "src/keys.ts"));
        assert_eq!(chain_phrase(&entries), "Edited src/keys.ts");
    }

    #[test]
    fn names_a_shared_location_when_the_calls_have_one() {
        assert_eq!(
            chain_phrase(&[
                c("Read", "packages/tui/src/a.ts"),
                c("Read", "packages/tui/src/b.ts")
            ]),
            "Read packages/tui/src"
        );
    }

    #[test]
    fn counts_rather_than_naming_one_arbitrary_target() {
        assert_eq!(
            chain_phrase(&[c("Edit", "a.ts"), c("Edit", "b.ts"), c("Edit", "c.ts")]),
            "Edited 3 files"
        );
    }

    #[test]
    fn never_presents_a_glob_as_a_location() {
        assert_eq!(
            chain_phrase(&[
                call("CodeSearch", "*.test.ts", 4, false, false),
                call("CodeSearch", "keys", 2, false, false)
            ]),
            "Explored"
        );
    }

    #[test]
    fn gives_a_lone_call_its_own_subject_whatever_shape_it_is() {
        assert_eq!(
            chain_phrase(&[call("Shell", "bun run check", 40, false, false)]),
            "Ran bun run check"
        );
        assert_eq!(
            chain_phrase(&[call("CodeSearch", "toolOutputView", 27, false, false)]),
            "Searched toolOutputView"
        );
    }

    #[test]
    fn falls_back_to_a_count_for_several_commands() {
        assert_eq!(
            chain_phrase(&[c("Shell", "a"), c("Shell", "b")]),
            "Ran 2 commands"
        );
    }

    #[test]
    fn never_reads_a_shared_command_prefix_as_a_location() {
        assert_eq!(
            chain_phrase(&[
                c("Shell", "cd /Users/me/repo && ls"),
                c("Shell", "cd /Users/me/repo && cat x")
            ]),
            "Ran 2 commands"
        );
    }

    #[test]
    fn names_the_act_not_the_navigation_in_front_of_it() {
        assert_eq!(
            chain_phrase(&[call(
                "Shell",
                "cd /Users/me/repo && bun run check",
                40,
                false,
                false
            )]),
            "Ran bun run check"
        );
    }

    #[test]
    fn a_run_that_did_the_same_thing_every_time_is_that_thing_not_a_count() {
        assert_eq!(
            chain_phrase(&[
                c("Shell", "cd /repo && bun run check"),
                c("Shell", "cd /elsewhere && bun run check")
            ]),
            "Ran bun run check"
        );
        assert_eq!(
            chain_phrase(&[c("Read", "src/keys.ts"), c("Read", "src/keys.ts")]),
            "Read src/keys.ts"
        );
    }

    #[test]
    fn still_says_something_for_an_unrecognised_tool() {
        assert_eq!(
            chain_phrase(&[c("mcp__thing__do", "x")]),
            "Called mcp__thing__do"
        );
    }
}

mod chain_phrase_long_chains {
    use super::*;

    fn reads(n: usize, dir: &str) -> Vec<ChainEntry> {
        (0..n)
            .map(|i| call("Read", &format!("{dir}/f{i}.ts"), 10, false, false))
            .collect()
    }

    fn many(n: usize, tool: &str, subject: impl Fn(usize) -> String) -> Vec<ChainEntry> {
        (0..n).map(|i| c(tool, &subject(i))).collect()
    }

    #[test]
    fn an_incidental_act_does_not_get_to_name_a_long_chain() {
        let mut entries = many(28, "CodeSearch", |i| format!("symbol{i}"));
        entries.extend(reads(37, "packages/coding-agent/src"));
        entries.push(c(
            "Edit",
            "packages/coding-agent/src/core/tools/subagent.ts",
        ));
        assert_eq!(chain_phrase(&entries), "Read packages/coding-agent/src");
    }

    #[test]
    fn a_tenth_of_the_calls_is_enough_to_keep_the_headline() {
        let mut entries = reads(9, "packages/tui/src");
        entries.push(c("Edit", "src/keys.ts"));
        assert_eq!(chain_phrase(&entries), "Edited src/keys.ts · 9 reads");
        let mut entries = reads(10, "packages/tui/src");
        entries.push(c("Edit", "src/keys.ts"));
        assert_eq!(chain_phrase(&entries), "Read packages/tui/src");
    }

    #[test]
    fn below_the_threshold_a_long_chain_behaves_exactly_as_before() {
        let mut entries = reads(6, "src");
        entries.push(c("Edit", "src/keys.ts"));
        assert_eq!(chain_phrase(&entries), "Edited src/keys.ts");
    }

    #[test]
    fn a_location_too_broad_to_mean_anything_becomes_a_count() {
        let mut entries = reads(20, "packages/tui/src");
        entries.extend(reads(20, "packages/ai/src"));
        assert_eq!(chain_phrase(&entries), "Read 40 files");
    }

    #[test]
    fn a_short_chain_keeps_its_shallow_location() {
        assert_eq!(
            chain_phrase(&[c("Write", "docs/a.md"), c("Write", "docs/b.md")]),
            "Edited docs"
        );
    }

    #[test]
    fn an_absolute_location_keeps_its_leading_slash() {
        assert_eq!(
            chain_phrase(&[
                c("Read", "/etc/nginx/a.conf"),
                c("Read", "/etc/nginx/b.conf")
            ]),
            "Read /etc/nginx"
        );
    }

    #[test]
    fn a_minority_headline_admits_the_largest_thing_it_left_out() {
        let mut entries = many(3, "Edit", |i| format!("packages/tui/src/x{i}.ts"));
        entries.extend(many(10, "Shell", |_| "bun run check".into()));
        entries.extend(reads(5, "packages/tui/src"));
        assert_eq!(
            chain_phrase(&entries),
            "Edited packages/tui/src · 10 commands"
        );
    }

    #[test]
    fn a_headline_that_covers_the_run_says_nothing_more() {
        let mut entries = many(10, "Edit", |i| format!("packages/tui/src/x{i}.ts"));
        entries.extend(reads(3, "packages/tui/src"));
        assert_eq!(chain_phrase(&entries), "Edited packages/tui/src");
    }

    #[test]
    fn a_footnote_sized_remainder_is_not_worth_the_width() {
        let mut entries = many(4, "Edit", |i| format!("src/x{i}.ts"));
        entries.extend(many(2, "Shell", |_| "check".into()));
        entries.extend(many(2, "Read", |i| format!("src/y{i}.ts")));
        entries.extend(many(2, "CodeSearch", |i| format!("sym{i}")));
        entries.extend(many(2, "WebFetch", |_| "https://example.com".into()));
        assert!(!chain_phrase(&entries).contains('·'));
    }

    #[test]
    fn a_chain_spread_thin_across_everything_still_names_its_largest_act() {
        let mut entries = many(50, "TodoWrite", |_| "plan".into());
        entries.extend(many(4, "Edit", |i| format!("src/x{i}.ts")));
        entries.extend(many(4, "Shell", |_| "check".into()));
        assert_eq!(chain_phrase(&entries), "Edited 4 files · 4 commands");
    }
}
