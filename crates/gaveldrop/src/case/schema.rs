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
    let mut schema = schemars::schema_for!(Case);
    close_the_fake_types(&mut schema);
    serde_json::to_string_pretty(&schema).unwrap_or_default()
}

/// Marks the scenario types closed, which `deny_unknown_fields` could not.
///
/// Those three types carry no `deny_unknown_fields` — a rule flattens its response, and serde
/// forbids the two together; `Match` omits it so a project can compose its own criterion on top.
/// So schemars derives no `additionalProperties: false` for them, and the editor stays silent
/// about a key the loader now refuses.
///
/// Silence would be the wrong half to keep. An unknown criterion leaves the match empty, and an
/// empty match is the catch-all: the rule answers every call and the rules after it are dead. The
/// editor is where that gets caught before it is written, so it has to say the same thing the
/// loader says.
///
/// Edits the `Schema` in place rather than a `serde_json::Value` converted from it: the
/// conversion re-keys the whole document alphabetically, which turns three added lines into a
/// three-hundred-line diff and reorders every tooltip in the editor.
fn close_the_fake_types(schema: &mut schemars::Schema) {
    let Some(defs) = schema
        .get_mut("$defs")
        .and_then(|defs| defs.as_object_mut())
    else {
        return;
    };

    for name in ["Scenario", "Rule", "Match"] {
        if let Some(serde_json::Value::Object(definition)) = defs.get_mut(name) {
            definition.insert("additionalProperties".to_string(), false.into());
        }
    }
}

/// Where the committed schema lives.
///
/// Resolved from the crate's manifest directory rather than the current working
/// directory, so the regeneration test behaves the same however cargo was invoked.
///
/// Only meaningful in a checkout. From an extracted package it points two levels above the
/// crate — inside `~/.cargo/registry/` — so anything that *writes* there has to establish it is
/// in this repository first, which the regeneration test does.
pub fn committed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/case.schema.json")
}

/// True when the schema path sits in a checkout of this repository.
///
/// The published crate carries `src/` and no `docs/`, so `CARGO_MANIFEST_DIR/../..` lands
/// outside the extracted tree. Without this the regeneration test read nothing, compared it to
/// the rendered schema, found a difference, and **wrote a file into somebody else's cargo
/// registry** before failing. A test that fails is a nuisance; a test that writes outside its
/// own tree on another machine is not.
///
/// `ARCHITECTURE.md` beside `docs/` is the marker rather than `docs/` alone, which could exist
/// up there by coincidence — and the coincidence would be the one case where the damage happens.
#[cfg(test)]
fn inside_the_repository(schema_path: &std::path::Path) -> bool {
    schema_path
        .parent()
        .and_then(std::path::Path::parent)
        .is_some_and(|root| root.join("ARCHITECTURE.md").is_file())
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

    #[test]
    fn the_editor_and_the_loader_refuse_the_same_keys_under_fake() {
        let rendered = render();
        let schema: serde_json::Value = serde_json::from_str(&rendered).unwrap();

        for (name, known) in [
            ("Scenario", gaveldrop_fake::Scenario::KEYS),
            ("Rule", gaveldrop_fake::Rule::KEYS),
            ("Match", gaveldrop_fake::Match::KEYS),
        ] {
            let definition = &schema["$defs"][name];

            assert_eq!(
                definition["additionalProperties"], false,
                "`{name}` cannot carry `deny_unknown_fields` — a rule flattens its response, and \
                 `Match` stays open so a project can compose its own criterion — so schemars \
                 derives nothing and the schema has to be closed by hand"
            );

            let mut described: Vec<&str> = definition["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("`{name}` must describe its properties"))
                .keys()
                .map(String::as_str)
                .collect();
            described.sort();

            assert_eq!(
                described, known,
                "the editor refuses what the schema describes and the loader refuses what \
                 `{name}::KEYS` lists. If the two ever disagree, one of them is wrong about a \
                 real case: the editor flags a legitimate key, or the loader accepts one nothing \
                 reads"
            );
        }
    }

    /// Regenerates the committed schema and fails when it has drifted.
    ///
    /// Intentionally writes into the repository rather than a tempdir: the schema is a
    /// committed artefact. When this fails, the format changed — rerun and commit the
    /// schema together with the format change, in the same commit.
    #[test]
    fn the_guard_tells_a_checkout_from_an_extracted_package() {
        let elsewhere = tempfile::tempdir().unwrap();
        let schema = elsewhere.path().join("docs/case.schema.json");

        assert!(
            !inside_the_repository(&schema),
            "no `ARCHITECTURE.md` above `docs/`, so this is not our checkout and nothing may be \
             written here"
        );

        std::fs::write(elsewhere.path().join("ARCHITECTURE.md"), "x").unwrap();
        assert!(
            inside_the_repository(&schema),
            "and with the marker in place the regeneration must still happen, or the drift check \
             is switched off in the one place it matters"
        );
    }

    #[test]
    fn the_committed_schema_is_up_to_date() {
        let path = committed_path();

        // From an extracted package this path is inside somebody else's cargo registry. There is
        // no committed schema to keep up to date there, and writing one would be vandalism.
        if !inside_the_repository(&path) {
            return;
        }

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
