# Changelog

Generated from the commit history by git-cliff. Do not edit by hand: run
`TAG=v0.1.1 mise run changelog:write` at release time.

## [0.1.1] - 2026-08-03

### Features

- *(cli)* Add the action, in this repository rather than its own (#85)
- *(core)* Let a case hide a tool the project fakes (#88)

### Bug fixes

- *(core)* Show what the subject wrote, not its first line (#87)

### Documentation

- Tick the release that is already out (#84)

## [0.1.0] - 2026-08-03

### Features

- *(fake)* Scaffold crate and normalize invocations
- *(fake)* Add rule matching, responses and scenarios
- *(fake)* Add persistent per-key call counter
- *(fake)* Add append-only call journal
- *(fake)* Add the four response modes
- *(fake)* Load and validate scenarios in one step
- *(fake)* Add the fake binary
- *(core)* Add the case format and its loader
- *(core)* Derive and commit the case JSON schema
- *(core)* Prepare an isolated environment per case
- *(core)* Observe a run through the process adapter
- *(core)* Evaluate expectations and locate every failure
- *(core)* Aggregate outcomes and render them as they finish
- *(core)* Discover cases from a project config and drive them
- *(cli)* Add the gaveldrop facade
- *(core)* Snapshot the isolated tree and diff it
- *(core)* Assert on the files the subject wrote
- *(core)* Extract structured events from stdout
- *(core)* Assert on structured events and their counts
- *(core)* Add the four named invariant shapes
- *(core)* Name invariants in the config and use them by name
- *(core)* Add the setup hook
- *(core)* Add the expect hook and document the protocol
- *(core)* Add the JSON Lines report and its merge
- *(core)* Render a self-contained HTML report
- *(cli)* Write the JSON and HTML reports
- *(conformance)* Add the adapter conformance kit
- *(core)* Assemble the shell command line with inert arguments
- *(core)* Choose the adapter from what the case declares
- *(core)* Hand every case a port nobody is listening on
- *(core)* Keep a subject alive and stop it without leaking it
- *(fake)* Answer from the same rules behind an HTTP door
- *(core)* Let a case invoke its subject more than once
- *(core)* Assert on the status, headers and body of a response
- *(core)* Invoke a living service across several steps
- *(core)* Assert on a value inside a JSON body
- *(core)* Assert on a JSON path in a response
- *(core)* Carry a named value from one exchange to the next
- *(core)* Assert that an exchange changed no files
- *(fake)* Shape a faked response through the render hook
- *(core)* Resolve an assertion path to its line
- *(core)* Write a JUnit report
- *(core)* Annotate a failure on the case's own line
- *(core)* Fail a run against the project's own thresholds
- *(core)* Split a suite across machines
- *(core)* Let a project refuse passthrough where it cannot work
- *(core)* List the cases for something other than a person
- *(cli)* Rerun what a save affected
- *(core)* Let a project run its suite through its own adapter (#70)
- *(core)* Let a case declare the variables its subject reads (#75)
- *(core)* Let a case hide a tool so absence is provable (#76)
- *(cli)* Show what the engine decided before each case (#77)

### Bug fixes

- *(fake)* Skip our own executable by identity, not by directory
- *(conformance)* Pin exactness to the adapter that controls its environment
- *(core)* Resolve a shell source against the project it belongs to
- *(core)* Decide readiness by what counts as an answer
- *(core)* Stop flooding a service while waiting for it
- *(core)* Serve the port before announcing it in the fixture
- *(core)* Refuse a fake with no catch-all when the case loads
- *(core)* Resolve project paths in a run command line
- *(core)* Refuse a fake key nothing reads instead of widening the match (#71)
- *(core)* Fail where a capture was declared, not two steps later (#73)
- *(core)* Keep the schema test inside this repository (#74)
- *(core)* Find the fake on PATH and say which install is missing (#80)

### Documentation

- Align commit convention with generated changelog
- Switch the whole repository to English
- Add AGENTS.md and point CLAUDE.md at it
- Document branch protection and mise setup
- Record that the fake engine is implemented
- Add a roadmap checklist by batch
- Record what the shell adapter cost the core
- Record the traps this repository has already sprung
- Add the traps found while building the web subject
- Record what the living subject cost the core
- Record where a case stops being data
- Stop saying only the fake engine exists
- Record what CI asked of decisions made earlier
- Record that the schema is already served
- Record the crates as published and correct what that made false (#78)
- Install the crate that has the binary in the CI job (#79)

