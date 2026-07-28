//! Environment variable names that link the core to the fake binary.
//!
//! These are the only channels between the two: the fake takes no arguments of
//! its own, because it receives exactly those of the binary it stands in for.

/// Path to the YAML file describing the scenario.
pub const SCENARIO: &str = "GAVELDROP_SCENARIO";

/// Directory where the call counter persists, one file per key.
pub const STATE: &str = "GAVELDROP_STATE";

/// Path to the call journal, opened in append mode.
pub const JOURNAL: &str = "GAVELDROP_JOURNAL";

/// Isolated directory of the running case. Passed on to project hooks.
pub const DIR: &str = "GAVELDROP_DIR";

/// Name of the running case. Passed on to project hooks, for their messages.
pub const CASE: &str = "GAVELDROP_CASE";
