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
    /// The structured events read from standard output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<crate::verdict::events::Event>,
    /// The files the subject created, modified or removed under the isolated root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<crate::iso::snapshot::FileEffect>,
    /// What this technology **alone** can produce. Anything observable of an arbitrary
    /// process already has a named field above; this is not a junk drawer.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, serde_json::Value>,
    /// The status a service answered with, absent when the subject answered no request.
    ///
    /// Kept apart from `exit`: an exit code is 0–255 and says whether a program succeeded, a status
    /// is three digits and says something else. A subject can have both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// The response headers, as they were received — spelling included.
    ///
    /// Normalising here would throw away what the server actually sent, and an observation should
    /// record that. Case-insensitive matching happens where the comparison does.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// The response body.
    ///
    /// Not `stdout`: for a service, `stdout` is its own logging, and the body is what it answered a
    /// request with. Asserting on one when you meant the other would be a quiet mistake.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// One entry per exchange, for a case that declared `steps:`.
    ///
    /// Nested rather than returned alongside, so an adapter reports everything through one value and
    /// the trait keeps its shape. The fields above still describe the run as a whole — for a service
    /// that is its own output and the files it wrote, which belong to no single exchange.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Observations>,
}
