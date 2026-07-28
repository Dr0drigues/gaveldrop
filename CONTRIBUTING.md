# Development rules

This document is deliberately short: rules nobody reads are not rules. It applies
to everyone — human or agent.

Read before touching the code: this, then `ARCHITECTURE.md`.

## The rule that governs the others

**The "Architecture Invariant" callouts in `ARCHITECTURE.md` are the contract.** A
change that breaks one is not a detail to quietly fix. There are only two ways out:

1. Drop the change.
2. Amend `ARCHITECTURE.md` **in the same commit**, writing down the reason.

There is no third way out. An invariant that erodes without anyone writing it down
is an invariant that never existed.

## Language

**Everything in this repository is in English.** Identifiers, format keywords, doc
comments, error messages, assertion messages, test names, commit messages,
documents. No exceptions, and therefore no boundary to remember.

## The rhythm

Test first, in this order, no exceptions:

1. Write the failing test.
2. **Run it, and check that it fails for the right reason.**
3. Write the minimal implementation that makes it pass.
4. Run it again.
5. Commit.

Step 2 is not decoration. A test that passes before the implementation exists
tests nothing, and you only find out at the first regression it should have
caught. Checking *why* it fails also catches the case where it fails to compile
over a typo.

Frequent, small commits. One commit is one behaviour.

## The three gates

No commit gets past these three without them passing:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

`-D warnings` is not negotiable. A warning tolerated once becomes a warning
tolerated always, and the noise ends up hiding the signal.

## Code conventions

### What we do not use

**No `async`.** The project spawns processes and reads files. A suite run is bound
by system calls, not by network waits. Pulling in an async runtime would be dead
weight on a binary whose startup time is precisely a design constraint.

**No `unsafe`.** `forbid` at the workspace level, not `deny`: the difference is
that it cannot be lifted locally.

**No speculative genericity.** A type parameter costs serde bounds, schemars
bounds, and compiler errors three times as long. It is only introduced with a
**second real implementor** in sight. `Rule` was generic in an early draft of the
plan; it no longer is, because the only consumer we could imagine declares its own
type anyway.

**No home-grown procedural macros.** Library derives are enough.

### What the machine guarantees

Two rules do not depend on goodwill. They live in the workspace lints, therefore in
`-D warnings`, therefore in CI:

```toml
[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"

[workspace.lints.clippy]
unwrap_used = "deny"
expect_used = "deny"
```

**No `unwrap()` and no `expect()`.** If a case really is impossible, lift the lint
locally — and the justification goes **inside the attribute**, not next to it:

```rust
#[expect(
    clippy::unwrap_used,
    reason = "the catch-all guarantees a rule always matches; Scenario::load checked it"
)]
```

`expect` rather than `allow`, because it also warns once the exemption becomes
unnecessary — a forgotten `allow` outlives the reason that created it
indefinitely. And "it's obvious" is not a reason.

The test exemption is declared once per crate, not once per module:

```rust
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        reason = "panicking is how a test reports failure"
    )
)]
```

**Every public item is documented** (`missing_docs`). This is not bureaucracy: for
the case format types, those docs **are** the tooltips seen in the editor, by way
of the JSON schema.

### Errors

**`thiserror` in libraries, `anyhow` in binaries.** A library returns errors a
caller can discriminate; a binary prints them and exits.

**Context goes in the variant's fields, not in a formatted string.** That is what
lets a caller react to the offending path instead of scraping it out of text.

```rust
// No — the caller can do nothing with this.
return Err(anyhow!("invalid scenario"));

// Yes — the path and the cause stay usable.
return Err(ScenarioError::Invalid { path: path.to_path_buf(), source });
```

**`#[error(...)]` messages start lowercase and carry no trailing period.** They
chain: `"scenario X is unreadable: line 4, column 2"`. A period in the middle of an
error chain breaks it into two limping sentences.

**An error message names the offender**: the path, the key, the value it got. And
where possible, it says **what to do**:

```rust
#[error(
    "scenario has no catch-all: add a `match: {{}}` rule last. \
     Without it an unexpected call would pass for an expected one."
)]
```

**No `Box<dyn Error>`** in a public signature.

### Naming

**No abbreviations in anything public.** A type is `Invocation`, not `Inv`; a
method is `args_joined`, not `args_j`. A local binding or a parameter may be short
when its scope fits on a screen — `inv`, `resp` — because readability gains more
than it loses there.

**Functions are imperative verbs**: `load`, `select`, `apply`, `record`. No `get_`
prefix on an accessor; that is the Rust convention.

**Modern module style**: `foo.rs` alongside a `foo/` directory, never `foo/mod.rs`.
The file name alone tells you where you are, without reading the path.

**`pub` is a commitment.** Anything outside the contract is `pub(crate)`. The
public contract is re-exported at the top of `lib.rs`, which makes it readable at a
glance.

### Comments

**No comments in files. Only doc comments are allowed** — `//!` at the top of a
module, `///` on an item. No `//` inside a function body, no `#` in a configuration
file.

Code reads through its own clarity: through apt names, short functions, and a
decomposition that makes the intent visible. An inline comment is a patch over a
failure to express something, and it is a patch that rots unnoticed — nothing
breaks when it becomes false.

**So the *why* moves up.** It does not disappear, it changes address:

| What you wanted to say | Where it goes now |
|---|---|
| why this module exists | its `//!` |
| why this item does that, what pitfall it avoids | its `///` |
| why this lint exemption | `reason = "…"` in the attribute |
| why this test exists, what it guards against | its name, and its assertion message |
| why this change | the PR description |
| why this structure | `ARCHITECTURE.md` |
| why this configuration, this dependency | this document |

The gain is not merely cosmetic: what used to be buried in a function body becomes
documentation **published** by `rustdoc`, and therefore read by whoever uses the
library, not only by whoever modifies it.

**And when an explanation has no item to attach to**, that is this rule's useful
signal: extract a named function whose `///` will carry it. The constraint pushes
towards decomposition rather than annotation, and that is exactly the right
reflex.

```rust
// No — the comment carries what the code fails to express.
// Skip our own directory, otherwise the fake would call itself forever.
let dirs = path.split(':').filter(|d| Some(*d) != skip);

// Yes — the name and its documentation carry the same thing, and rustdoc publishes it.
/// Looks `bin` up in `PATH`, **skipping** `skip_dir`.
///
/// Skipping our own directory is what stops passthrough from calling itself
/// forever: the fake sits first on `PATH` under precisely the name of the binary
/// it stands in for.
pub fn real_binary_in(bin: &str, path: &str, skip_dir: Option<&Path>) -> Option<PathBuf>
```

**Doc examples are compiled** by `cargo test`. They are the only kind of
documentation that cannot lie — prefer them to a prose explanation when both are
possible.

**Every module opens with a `//!` saying why it exists**, not what it contains —
`rustdoc` already generates the item list.

### Formatting

`rustfmt` defaults, **no `rustfmt.toml`**. A formatting config file is a standing
invitation to argue about line width. If deviating ever becomes truly necessary, it
will be a written decision, not a setting slipped in along the way.

## Writing tests

**A test name is a sentence you will read in a failure report.** It describes the
expected behaviour, not the function under test.

```rust
// No
fn test_catch_all() { … }

// Yes
fn catch_all_responds_and_flags_itself_in_the_journal() { … }
```

**One test per behaviour**, not one catch-all test per function. When a test
fails, its name alone should tell you what is broken.

**When a test guards against a specific pitfall, say so in the assertion
message.** It is a string, not a comment — and it is what you will read at the
moment the test fails, which is the only moment that counts:

```rust
assert_eq!(
    found, real_dir.join("git"),
    "without skipping our own directory, the fake would call itself forever"
);
```

**The three levels do not replace one another** — see `ARCHITECTURE.md`,
"Cross-Cutting Concerns":

- **unit tests**, closest to the code;
- **the conformance kit**, at the adapter boundary — this is gaveldrop's own
  guarantee;
- **real consumers**, outside this repository — that is where the
  non-regression proof lives, and it is never repatriated here.

## The case format

**No logic in the YAML.** Any proposal that adds a conditional, a loop, an
interpolation or a computation to the format is rejected by default. A case file
holds only facts; the moment something has to be decided, it moves out into an
executable through a hook.

This is the easiest rule to break with good intentions, because every single
addition looks reasonable on its own. The cumulative result is a failed
programming language written in YAML.

**The committed schema is regenerated by a test**, never hand-edited. If the
regeneration test fails, the format changed: rerun the generation and commit the
schema together with the format change, in the same commit.

**Three hooks.** A fourth is a decision justified in `ARCHITECTURE.md`, not a drift
noticed six months later.

## Dependencies

**A new dependency is justified in writing, in this document's "The foundation"
section.** Not in a conversation, not only in a PR: somewhere you find again while
wondering why it is there, and that ages with the code rather than with the
history.

**Versions align with the prototype's** where the dependency exists there — two
different resolutions in one build graph is a cost paid for nothing.

**`gaveldrop-fake` depends on no other crate in the repository.** Architecture
invariant, and the only one the compiler will not remind you of.

## Commits

**One line, in English, in Conventional Commits format. No body.**

```
feat(fake): add persistent per-key call counter
fix(core): skip our own dir when resolving the real binary
docs: document the foundation rationale
ci: run tests on macOS too
```

Shape: `type(scope): subject`. Imperative subject, lowercase, no trailing period,
72 characters at most. A breaking change is marked `feat(fake)!:`.

Types: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `chore`, `ci`, `build`.
Scopes: `fake`, `core`, `cli`, `conformance`. `docs` and `ci` are types, not
scopes — `docs:` with no scope.

**The format is not decoration: the changelog is generated from the commits**, by
`git-cliff`, and the messages are validated in CI by `committed`. Two concrete
consequences:

- **A commit subject is a release-notes line.** Write it for whoever reads the
  changelog, not for whoever rereads the diff. `add persistent per-key call
  counter` reads in release notes; `wip counter stuff` does not.
- **A breaking change is marked with `!`**, never with a `BREAKING CHANGE:`
  footer — there is no body to put it in. `feat(fake)!: …` is enough for
  `cargo-release` to bump the major version.

Two of these rules are machine-checked and one is not. `committed` rejects an
unknown type, an unknown scope, a trailing period, a non-imperative subject, a
`wip`/`fixup` message, and a subject over 72 characters. It does **not** enforce a
lowercase subject: its `subject_capitalized` setting can require capitalisation or
skip the check, but cannot require the opposite. So lowercase stays a convention
you hold yourself. Left that way on purpose — a second, hand-rolled validation
mechanism alongside a real tool is exactly the kind of thing that rots, and the
failure mode is cosmetic since `git-cliff` capitalises for display anyway.

**So the *why* does not go in the commit.** It goes in the PR description, and for
whatever is durable, in the documentation — see the table in the "Comments"
section. The distinction is sharp and worth remembering:

- the **why of the change** is circumstantial → PR description;
- the **why of the code** is durable → `///`, `ARCHITECTURE.md`,
  `CONTRIBUTING.md`.

A git history is an index, not a journal. What you look for in it is *when a thing
appeared* — and for that, one line beats a paragraph.

## Git flow

**Never commit straight to `main`.** One branch, one PR, green CI before merging.
Including solo — and especially solo: on a one-developer project, **CI is the only
reviewer**. Bypassing it means no longer being reviewed at all.

This one is not on trust: `main` is protected on GitHub, and the protection applies
to administrators too. See "Branch protection" below for what it refuses.

Branch naming: `<type>/<kebab-description>`, using the same type vocabulary as
commits.

```
feat/call-counter
fix/passthrough-recursion
docs/development-rules
```

One branch per plan task. The task breakdown already has the right grain — there is
no reason to invent a second one.

**Rebase-and-merge**, for a linear history with no merge commits. Since commit
messages are single lines and a task produces one commit, squash or not changes
almost nothing — rebasing is simply what keeps the graph readable.

**The PR description carries the why of the change.** It is what replaced the
commit body: the pitfall avoided, the option discarded, the constraint endured.
Including solo — that is where you find the reasoning again six months later,
without cluttering the history.

## The foundation

Configuration files carry no comments — this section carries their reasons. When
you change one of them, you change this section too.

**License: `MIT OR Apache-2.0`**, the Rust ecosystem's dual standard. It sits at
the repository level: no license header to copy into source files. The text of
`LICENSE-APACHE` comes from `apache.org`; it was not retyped from memory.

### `rust-toolchain.toml`

Toolchain **pinned to 1.97**, with `rustfmt` and `clippy` as components so CI needs
no install step.

The code's real floor is **1.88** — let chains are only stable from there, and only
in edition 2024. It is declared by `rust-version` in the manifest, which yields a
clear message instead of an incomprehensible syntax error. We pin higher to be
reproducible rather than merely compatible.

Acknowledged: that floor is **not** verified by CI, which runs on the pinned
version. Raising the pinned version is a one-line commit — to be done when a reason
calls for it, not reflexively at every Rust release.

### `.github/workflows/ci.yml`

The three gates, checked by a machine. On a solo project, that is the only
reviewer.

`RUSTFLAGS: -D warnings` at workflow level: a warning tolerated once becomes a
warning tolerated always, and the noise ends up hiding the signal. This is also
what turns `missing_docs = "warn"` into an effective refusal.

`concurrency` with `cancel-in-progress`: a new push to a branch cancels that
branch's previous check. The latest one is what counts, not the queue.

**No toolchain action.** `rust-toolchain.toml` is pinned, and rustup honours it by
itself on the first cargo call — an action would only duplicate the decision, with
a risk of contradicting it. A step does print the versions, so that a
toolchain-caused failure stays distinguishable from a code-caused one.

**Two platforms, two asymmetric jobs.** Formatting and clippy run on Linux only:
they do not depend on the platform, and paying twice for the same result buys
nothing. Tests run on Linux **and** macOS, because isolation does depend on the
platform — symlinks, permissions, config directories — and because the shell
increment will target macOS paths.

### `Cargo.toml`

Lints live at the workspace level, and every crate must declare
`[lints] workspace = true` to inherit them. Without that block everything compiles
and nothing is checked: it is a silent trap, and no warning exists to flag it.

Dependency versions align with the prototype's where the dependency exists there.

`glob` is there for one job: expanding the `cases:` pattern from the project config. The
standard library has no glob, and hand-rolling one over `read_dir` would be a recursive
walk plus pattern matching — more code than the dependency, and code whose edge cases we
would own.

`tempfile` is a **regular** dependency of `gaveldrop`, not a dev one. `Isolation` owns a
`TempDir`, and that ownership is what removes the isolated directory when the case is
done. Listing it under `[dev-dependencies]` compiles under `cargo test`, which makes
dev-dependencies available, and then fails a plain `cargo build` — a trap worth naming
because the test gate alone does not catch it. The clippy gate does.

The version lives once, in `[workspace.package]`, and every crate inherits it with
`version.workspace = true`. The crates are released together, so a per-crate version
would be three places to forget.

### Release tooling

Three Rust tools, one job each. All three are available as prebuilt Homebrew
formulae, so nobody has to compile them.

| Tool | Job | Config |
|---|---|---|
| `committed` | validates commit messages, in CI on every PR | `committed.toml` |
| `git-cliff` | generates `CHANGELOG.md` from the commits | `cliff.toml` |
| `cargo-release` | bumps the version, tags, calls `git-cliff` | `release.toml` |

Three tools rather than one is the accepted cost of staying in the Rust ecosystem.
Commitizen would have covered all three from a single config, but it is a Python
tool, and each of these three is better at its own job — `git-cliff` has real
templating, and `cargo-release` understands workspace publishing order.

`cliff.toml` deliberately skips `chore`, `ci`, `build`, `test` and `style` from the
changelog: they are not release notes. `protect_breaking_commits` keeps a breaking
change visible even when its type would otherwise be skipped.

`release.toml` has `publish = false` and `push = false`. Publishing is the
distribution increment's job; turning it on is a deliberate two-line change, not
something to discover by accident. `allow-branch = ["main"]` refuses to release from
anywhere else. Check the whole flow with `cargo release version patch`, which is a
dry run unless you pass `--execute`.

**There is no `CHANGELOG.md` yet, on purpose.** It is generated by the
`pre-release-hook` at the first real release. Committing one now would mean a file
that goes stale on every commit until then, which is worse than no file.

One accepted risk: `.github/workflows/commits.yml` references
`crate-ci/committed@master`, unpinned. That is what the tool's own project
documents and uses for itself, and pinning would mean guessing a tag. Worth
revisiting if the action ever starts publishing releases.

### Branch protection on `main`

Configured on GitHub, not in a file, so it is written down here instead.

| Setting | Value |
|---|---|
| Pull request required | yes |
| Approving reviews required | 0 |
| Required checks | `Les trois portes (Linux)`, `Tests (macOS)`, `Conventional Commits` |
| Branch up to date before merging | yes |
| Applies to administrators | **yes** |
| Linear history required | yes |
| Force pushes | refused |
| Branch deletion | refused |
| Conversations must be resolved | yes |

**Zero required approvals**, because you cannot approve your own pull request:
requiring one would deadlock a solo project. The pull request itself is still
mandatory, which is what makes CI run.

**It applies to administrators.** That is the whole point — a protection the owner
can step around is a suggestion. The cost is real: rewriting history on `main` is
now impossible without temporarily lifting the protection, so a badly worded commit
has to be lived with or fixed through a revert.

**Linear history required** turns the rebase-and-merge rule into a mechanism rather
than a habit.

Read the live settings with
`gh api repos/Dr0drigues/gaveldrop/branches/main/protection`.

### `.mise.toml`

Pins the three release tools and defines the tasks. `mise run gates` runs the three
gates in parallel; `mise tasks` lists the rest.

**It deliberately does not pin Rust.** `rust-toolchain.toml` is the single authority
there, and rustup honours it everywhere — locally, in CI, and for rust-analyzer. Two
authorities over the same version is a drift waiting to happen.

`cargo-release` is pinned to **1.1.2, not the 1.1.3 that crates.io serves**. There is
no GitHub release for 1.1.3, so no prebuilt binary exists for mise to fetch; the last
tag is `v1.1.2`. Pinning 1.1.3 would force everyone to compile it. The `release.toml`
config was dry-run against both versions and behaves identically.

The task definitions duplicate the three gate commands, which also live in
`ci.yml`. Accepted: CI keeps one step per gate so a failure shows which gate broke,
and the commands are short and stable. Unifying would mean installing mise in CI to
gain little.

## What is not a rule here

No test coverage threshold. A numeric coverage target is satisfied by writing tests
that catch nothing, and this project is precisely a tool for catching things — it
would be an odd way to start.
