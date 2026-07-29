//! Reading reports back, and consolidating several into one.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::Outcome;
use crate::report::Report;

/// What can go wrong while reading a report back.
#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    /// A report file could not be read.
    #[error("reading the report {path}: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// A line was not a readable outcome.
    #[error("line {line} is not a readable outcome: {source}")]
    Line {
        /// Which line, 1-indexed.
        line: usize,
        /// The underlying failure.
        #[source]
        source: serde_json::Error,
    },
}

/// Reads a JSON Lines report.
///
/// A malformed line is an **error naming its number**, never a skip. A consolidated report feeds
/// gating, so an outcome silently dropped would move the score without anyone noticing. Blank
/// lines are tolerated, because concatenating files whose last line lacks a newline is the normal
/// accident.
pub fn read(reader: impl BufRead) -> Result<Report, MergeError> {
    let mut outcomes = Vec::new();

    for (index, line) in reader.lines().enumerate() {
        let line = line.map_err(|source| MergeError::Io {
            path: PathBuf::from("(stream)"),
            source,
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let outcome: Outcome = serde_json::from_str(&line).map_err(|source| MergeError::Line {
            line: index + 1,
            source,
        })?;
        outcomes.push(outcome);
    }

    Ok(Report::from(outcomes))
}

/// Reads and merges several report files, in the order given.
pub fn read_all(paths: &[PathBuf]) -> Result<Report, MergeError> {
    let mut merged = Vec::new();

    for path in paths {
        merged.extend(read_one(path)?.outcomes);
    }

    Ok(Report::from(merged))
}

/// One report file, with its path in any error.
fn read_one(path: &Path) -> Result<Report, MergeError> {
    let file = std::fs::File::open(path).map_err(|source| MergeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    read(std::io::BufReader::new(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHARD_A: &str = "{\"name\":\"a\",\"weight\":5,\"allow_fail\":false,\"passed\":true,\"diffs\":[],\"unexpected_calls\":[]}\n";
    const SHARD_B: &str = "{\"name\":\"b\",\"weight\":3,\"allow_fail\":false,\"passed\":false,\"diffs\":[],\"unexpected_calls\":[]}\n";

    #[test]
    fn one_shard_reads_back_into_a_report() {
        let report = read(SHARD_A.as_bytes()).unwrap();

        assert_eq!(report.summary().total, 1);
        assert_eq!(report.summary().score, 5);
    }

    #[test]
    fn concatenated_shards_merge_and_the_summary_is_recomputed() {
        let concatenated = format!("{SHARD_A}{SHARD_B}");
        let summary = read(concatenated.as_bytes()).unwrap().summary();

        assert_eq!(
            summary.total, 2,
            "merging is `cat`, because nothing but outcomes was ever written"
        );
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.score, 5);
        assert_eq!(summary.max_score, 8);
    }

    #[test]
    fn a_blank_line_between_shards_is_tolerated() {
        let sloppy = format!("{SHARD_A}\n{SHARD_B}");
        assert_eq!(read(sloppy.as_bytes()).unwrap().summary().total, 2);
    }

    #[test]
    fn a_malformed_line_names_its_number_rather_than_being_skipped() {
        let broken = format!("{SHARD_A}not json at all\n{SHARD_B}");
        let error = read(broken.as_bytes()).unwrap_err();

        assert!(
            error.to_string().contains('2'),
            "a report is consolidated for gating, so a line silently dropped would move the score \
             without anyone noticing. Name the line: {error}"
        );
    }

    #[test]
    fn several_files_merge_in_the_order_given() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("a.jsonl");
        let second = dir.path().join("b.jsonl");
        std::fs::write(&first, SHARD_A).unwrap();
        std::fs::write(&second, SHARD_B).unwrap();

        let report = read_all(&[first, second]).unwrap();
        let names: Vec<&str> = report
            .outcomes
            .iter()
            .map(|outcome| outcome.name.as_str())
            .collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn a_missing_file_names_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let error = read_all(&[dir.path().join("absent.jsonl")]).unwrap_err();
        assert!(error.to_string().contains("absent.jsonl"));
    }
}
