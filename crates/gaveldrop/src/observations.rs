//! What a run produced, normalised so that expectations behave identically whatever the
//! technology.

use std::collections::BTreeMap;

use gaveldrop_fake::Call;
use serde::{Deserialize, Serialize};

/// Everything observed about one run.
///
/// Entirely serialisable data — no file handle, no closure, no live object. That is the
/// constraint that keeps an adapter able to live in another language one day without
/// reshaping this contract.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Observations {
    /// The subject's exit code.
    pub exit: i32,
    /// Everything the subject wrote on standard output.
    pub stdout: String,
    /// Everything the subject wrote on standard error.
    pub stderr: String,
    /// The call journal, as the fake left it.
    pub calls: Vec<Call>,
    /// The files the subject created, modified or removed under the isolated root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<crate::iso::snapshot::FileEffect>,
    /// What this technology **alone** can produce. Anything observable of an arbitrary
    /// process already has a named field above; this is not a junk drawer.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, serde_json::Value>,
}
