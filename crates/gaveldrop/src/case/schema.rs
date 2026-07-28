//! The JSON schema for the case format.
//!
//! **Derived from the type, never hand-written.** A hand-written schema would lie at the
//! first change of shape, and this one is load-bearing: it is what makes the format safe
//! to write by hand and to generate with an agent, and it is what carries the doc
//! comments into editor tooltips.

use std::path::PathBuf;

use crate::Case;

/// Renders the schema as pretty-printed JSON.
pub fn render() -> String {
    let schema = schemars::schema_for!(Case);
    serde_json::to_string_pretty(&schema).unwrap_or_default()
}

/// Where the committed schema lives.
///
/// Resolved from the crate's manifest directory rather than the current working
/// directory, so the regeneration test behaves the same however cargo was invoked.
pub fn committed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/case.schema.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_describes_the_case_type() {
        let rendered = render();
        let schema: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(schema["title"], "Case");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|key| key == "name"));
        assert!(required.iter().any(|key| key == "expect"));
    }

    #[test]
    fn the_schema_carries_the_doc_comments_as_descriptions() {
        let rendered = render();
        assert!(
            rendered.contains("Reports sort failures by it"),
            "doc comments must reach the schema: they are the tooltips seen in the \
             editor by whoever writes a case, which is the only user manual this format \
             has"
        );
    }

    #[test]
    fn the_schema_refuses_unknown_keys_outside_setup() {
        let rendered = render();
        let schema: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            schema["additionalProperties"], false,
            "a misspelled top-level key must be flagged in the editor, not silently \
             ignored into a case that asserts nothing"
        );
    }

    #[test]
    fn setup_stays_open_because_the_core_does_not_own_its_vocabulary() {
        let rendered = render();
        let schema: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        let setup = &schema["$defs"]["Setup"];

        assert!(
            setup.is_object(),
            "Setup must appear in $defs; schema was:\n{rendered}"
        );
        assert_ne!(
            setup["additionalProperties"], false,
            "`setup` is deliberately open: everything beyond `run` and `exec` belongs to \
             the project and travels to its hook, so the schema must not reject it"
        );
    }

    /// Regenerates the committed schema and fails when it has drifted.
    ///
    /// Intentionally writes into the repository rather than a tempdir: the schema is a
    /// committed artefact. When this fails, the format changed — rerun and commit the
    /// schema together with the format change, in the same commit.
    #[test]
    fn the_committed_schema_is_up_to_date() {
        let path = committed_path();
        let rendered = render();
        let previous = std::fs::read_to_string(&path).unwrap_or_default();

        if previous != rendered {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, &rendered).unwrap();
            panic!(
                "{} was out of date and has just been rewritten. Review the diff and \
                 commit it together with the format change.",
                path.display()
            );
        }
    }
}
