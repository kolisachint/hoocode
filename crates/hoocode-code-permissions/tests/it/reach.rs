//! The clearance card's REACH line (M6, spec §4 c1, §8.11): what a gated
//! call can reach, from the call's real data. Paths are never renamed.

use hoocode_code_permissions::reach_for;
use serde_json::json;

fn shell(command: &str) -> hoocode_code_permissions::Reach {
    reach_for("Shell", &json!({"command": command}))
}

#[test]
fn pipe_counts_the_joined_programs() {
    let reach = shell("printf 'alpha\\nbeta\\ngamma\\n' | sort -r");
    assert!(!reach.read.on);
    assert!(!reach.write.on);
    assert!(!reach.network.on);
    assert!(reach.pipe.on);
    assert_eq!(reach.pipe.qualifier.as_deref(), Some("2 programs"));
}

#[test]
fn read_names_the_command_path() {
    let reach = shell("wc -l src/wrap.rs");
    assert!(reach.read.on);
    assert_eq!(reach.read.qualifier.as_deref(), Some("src/wrap.rs"));
    assert!(!reach.write.on);
    assert!(!reach.network.on);
    assert!(!reach.pipe.on);
}

#[test]
fn or_or_is_not_a_pipe() {
    let reach = shell("test -f a.txt || echo missing");
    assert!(!reach.pipe.on);
}

#[test]
fn redirect_names_the_write_target() {
    let reach = shell("echo hello > out/notes.txt");
    assert!(reach.write.on);
    assert_eq!(reach.write.qualifier.as_deref(), Some("out/notes.txt"));
}

#[test]
fn network_programs_and_urls_reach_the_network() {
    assert!(shell("curl https://example.test/x").network.on);
    assert!(shell("echo https://example.test/x").network.on);
    assert!(!shell("echo hi").network.on);
}

#[test]
fn quoted_pipes_do_not_join() {
    let reach = shell("echo 'a | b'");
    assert!(!reach.pipe.on);
}

#[test]
fn edit_reads_and_writes_its_path() {
    let reach = reach_for("Edit", &json!({"path": "src/app.ts"}));
    assert!(reach.read.on);
    assert_eq!(reach.read.qualifier.as_deref(), Some("src/app.ts"));
    assert!(reach.write.on);
    assert_eq!(reach.write.qualifier.as_deref(), Some("src/app.ts"));
}

#[test]
fn write_only_writes_its_path() {
    let reach = reach_for("Write", &json!({"file_path": "tools/util.py"}));
    assert!(!reach.read.on);
    assert!(reach.write.on);
    assert_eq!(reach.write.qualifier.as_deref(), Some("tools/util.py"));
}

#[test]
fn web_tools_reach_the_network() {
    for tool in ["WebFetch", "WebSearch"] {
        let reach = reach_for(tool, &json!({}));
        assert!(!reach.read.on);
        assert!(!reach.write.on);
        assert!(reach.network.on);
        assert!(!reach.pipe.on);
    }
}

#[test]
fn unknown_tools_reach_nothing() {
    let reach = reach_for("ShellKill", &json!({}));
    assert!(!reach.read.on);
    assert!(!reach.write.on);
    assert!(!reach.network.on);
    assert!(!reach.pipe.on);
}
