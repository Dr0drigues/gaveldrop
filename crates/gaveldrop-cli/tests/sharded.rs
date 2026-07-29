//! A sharded run, merged, compared against running everything at once.
//!
//! The ROADMAP words this as "report merging exercised by a real sharded run", and it is a test rather
//! than a feature. Two decisions meet here and only a real run can show whether they hold: the report
//! stores outcomes and computes its summary, decided in lot 4 so shards could be concatenated; and the
//! partition is `index modulo of`, decided in lot 7 so no coordinator is needed.
//!
//! If sharding is not a partition, this is what says so.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "panicking is how a test reports failure; an integration test file is its own \
              crate, so the library's cfg_attr does not cover it"
)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use assert_cmd::cargo::cargo_bin;
use gaveldrop::report::merge;

/// A project of `count` cases, half of which fail, with weights that differ.
///
/// Differing weights matter: equal ones would let a wrong partition produce the right score by
/// accident, which is exactly the bug this file exists to catch.
fn project(count: usize) -> tempfile::TempDir {
    let fake = cargo_bin("gaveldrop-fake");
    assert!(
        fake.is_file(),
        "{} is missing. Run `cargo test --workspace` rather than testing this crate alone.",
        fake.display()
    );

    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("tests/cases")).unwrap();
    fs::write(
        dir.path().join("gaveldrop.yaml"),
        "cases: tests/cases/**/*.yaml\n",
    )
    .unwrap();

    for at in 0..count {
        let exit = if at % 2 == 0 { 0 } else { 7 };
        fs::write(
            dir.path().join(format!("tests/cases/case-{at:02}.yaml")),
            format!(
                "name: case-{at:02}\nweight: {}\nsetup:\n  run: [\"sh\", \"-c\", \"exit {exit}\"]\nexpect:\n  exit_code: 0\n",
                at + 1
            ),
        )
        .unwrap();
    }

    dir
}

/// Runs gaveldrop in `dir`, writing JSON Lines to `into`, optionally for one shard.
fn run(dir: &tempfile::TempDir, into: &str, shard: Option<&str>) -> PathBuf {
    let report = dir.path().join(into);
    let mut command = Command::new(cargo_bin("gaveldrop"));
    command
        .current_dir(dir.path())
        .arg("--report-json")
        .arg(&report);

    if let Some(slice) = shard {
        command.arg("--shard").arg(slice);
    }

    command.output().unwrap();
    report
}

#[test]
fn three_shards_merged_equal_one_unsharded_run() {
    let dir = project(10);

    let whole = merge::read_all(&[run(&dir, "whole.jsonl", None)]).unwrap();
    let shards: Vec<PathBuf> = (0..3)
        .map(|at| run(&dir, &format!("shard-{at}.jsonl"), Some(&format!("{at}/3"))))
        .collect();
    let merged = merge::read_all(&shards).unwrap();

    let (one, many) = (whole.summary(), merged.summary());

    assert_eq!(
        one.total, many.total,
        "a case in two shards would be counted twice; a case in none would be silently untested. \
         Equal totals is the property that says the partition is one"
    );
    assert_eq!(
        one.score, many.score,
        "the weights differ per case, so an accidental partition cannot produce the right score by \
         coincidence"
    );
    assert_eq!(one.max_score, many.max_score);
    assert_eq!(one.passed, many.passed);
    assert_eq!(one.failed, many.failed);
}

#[test]
fn every_case_appears_exactly_once_across_the_shards() {
    let dir = project(7);

    let shards: Vec<PathBuf> = (0..3)
        .map(|at| run(&dir, &format!("shard-{at}.jsonl"), Some(&format!("{at}/3"))))
        .collect();

    let mut names: Vec<String> = merge::read_all(&shards)
        .unwrap()
        .outcomes
        .into_iter()
        .map(|outcome| outcome.name)
        .collect();
    names.sort();
    let before = names.len();
    names.dedup();

    assert_eq!(
        names.len(),
        before,
        "a name appearing twice means overlapping shards"
    );
    assert_eq!(
        names.len(),
        7,
        "seven cases across three shards, with a remainder — the case count not dividing evenly is \
         the arrangement a modulo partition has to get right"
    );
}

#[test]
fn a_shard_writes_no_summary_line_which_is_what_makes_cat_work() {
    let dir = project(4);
    let shard = run(&dir, "shard.jsonl", Some("0/2"));
    let text = fs::read_to_string(&shard).unwrap();

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("every line must be one outcome and nothing else: {error} in {line}")
        });
        assert!(
            parsed.get("name").is_some(),
            "a summary line among the outcomes would be counted as a case by whoever concatenated \
             the shards, and the merged totals would be wrong in a way nothing detects: {line}"
        );
    }
}

#[test]
fn concatenating_the_files_by_hand_gives_the_same_report() {
    let dir = project(6);

    let shards: Vec<PathBuf> = (0..2)
        .map(|at| run(&dir, &format!("shard-{at}.jsonl"), Some(&format!("{at}/2"))))
        .collect();

    let joined = dir.path().join("joined.jsonl");
    let mut text = String::new();
    for shard in &shards {
        text.push_str(&fs::read_to_string(shard).unwrap());
    }
    fs::write(&joined, text).unwrap();

    assert_eq!(
        merge::read_all(&[joined]).unwrap().summary(),
        merge::read_all(&shards).unwrap().summary(),
        "`cat shard-*.jsonl > all.jsonl` is what a CI job actually does, and it has to give the \
         same answer as reading the files one by one"
    );
}
