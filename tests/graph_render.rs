//! Integration test: build a small DAG, render it to plain text, snapshot-compare
//! the glyph layout. Run with `cargo test --test graph_render -- --nocapture`
//! to see the actual output for visual debugging.

use gsc::git::{Commit, RefName};
use gsc::graph::{layout, row_spans};
use gsc::ui::theme;

fn commit(hash: &str, parents: &[&str], subject: &str, refs: Vec<RefName>) -> Commit {
    Commit {
        hash: hash.to_string(),
        short_hash: hash[..hash.len().min(7)].to_string(),
        parents: parents.iter().map(|s| s.to_string()).collect(),
        subject: subject.to_string(),
        author: "tester".to_string(),
        time: 0,
        refs,
    }
}

/// Strip ratatui Spans into plain text for snapshot comparison.
fn render_to_text(commits: &[Commit]) -> String {
    let theme = theme::current();
    let rows = layout(commits);
    let mut out = String::new();
    for row in &rows {
        let commit = &commits[row.commit_idx];
        let spans = row_spans(row, commit, &theme, false, 0, usize::MAX);
        for s in spans {
            out.push_str(&s.content);
        }
        out.push('\n');
    }
    out
}

#[test]
fn renders_linear_history() {
    let cs = vec![
        commit("ccc", &["bbb"], "third", vec![]),
        commit("bbb", &["aaa"], "second", vec![]),
        commit("aaa", &[], "first", vec![]),
    ];
    let out = render_to_text(&cs);
    println!("\n{out}");
    // Each line should have ●  at the start.
    for line in out.lines() {
        assert!(line.starts_with('●'), "line should start with ●: {line:?}");
    }
}

#[test]
fn renders_simple_merge() {
    // Topology:
    //   c (merge of b, x)
    //   b -> a
    //   x -> a
    //   a
    let cs = vec![
        commit("ccc", &["bbb", "xxx"], "merge", vec![]),
        commit("bbb", &["aaa"], "feature work", vec![]),
        commit("xxx", &["aaa"], "side branch", vec![]),
        commit("aaa", &[], "init", vec![]),
    ];
    let out = render_to_text(&cs);
    println!("\n{out}");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 4);
    // Row 0 (merge) should have a spawning corner on lane 1
    assert!(
        lines[0].contains('╮') || lines[0].contains('╭'),
        "merge row should show a spawn corner: {:?}",
        lines[0]
    );
    // Row 3 (a) should absorb a lane
    assert!(
        lines[3].contains('╯') || lines[3].contains('╰'),
        "absorption row should show an absorb corner: {:?}",
        lines[3]
    );
}

#[test]
fn renders_branch_and_remote_pills() {
    // Three commits with various ref combinations:
    //   newest: HEAD on main, also at origin/main, also tagged v1.0
    //   middle: a local-only feature branch tip
    //   oldest: a remote-only origin/abandoned branch (rare, but valid)
    let cs = vec![
        commit(
            "aaaaaa",
            &["bbbbbb"],
            "release v1.0",
            vec![
                RefName::HeadAt("main".into()),
                RefName::RemoteBranch("origin/main".into()),
                RefName::Tag("v1.0".into()),
            ],
        ),
        commit(
            "bbbbbb",
            &["cccccc"],
            "wip on feature",
            vec![RefName::LocalBranch("feature".into())],
        ),
        commit(
            "cccccc",
            &[],
            "abandoned branch tip",
            vec![RefName::RemoteBranch("origin/abandoned".into())],
        ),
    ];
    let out = render_to_text(&cs);
    println!("\n{out}");

    let lines: Vec<&str> = out.lines().collect();
    // HEAD pill should appear on the newest commit
    assert!(lines[0].contains("◉ main"), "expected HEAD pill on row 0: {:?}", lines[0]);
    // `synced` chip should appear when local + remote co-exist
    assert!(lines[0].contains("synced"), "expected synced chip on row 0: {:?}", lines[0]);
    // Tag pill
    assert!(lines[0].contains("▸ v1.0"), "expected tag pill on row 0: {:?}", lines[0]);
    // Local-only branch
    assert!(lines[1].contains("⎇ feature"), "expected local pill on row 1: {:?}", lines[1]);
    assert!(
        !lines[1].contains("synced"),
        "row 1 has no remote, no synced chip: {:?}",
        lines[1]
    );
    // Remote-only branch — ↓ + origin/<name> indicates "would pull"
    assert!(
        lines[2].contains("↓ origin/abandoned"),
        "expected remote-only pill on row 2: {:?}",
        lines[2]
    );
}

#[test]
fn renders_long_diamond() {
    //   m (merges b1 and b2)
    //   b2 -> mid
    //   b1 -> mid
    //   mid -> root
    //   root
    let cs = vec![
        commit("mmm", &["b22", "b11"], "merge diamond", vec![]),
        commit("b22", &["mid"], "right side", vec![]),
        commit("b11", &["mid"], "left side", vec![]),
        commit("mid", &["root"], "common ancestor", vec![]),
        commit("root", &[], "init", vec![]),
    ];
    let out = render_to_text(&cs);
    println!("\n{out}");
}
