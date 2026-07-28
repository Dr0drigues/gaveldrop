# Instructions for coding agents

A test engine where **one case is one YAML file**. A case describes how to invoke a
program, how its dependencies must respond, and what the result must contain.

## Read before changing anything

1. **`CONTRIBUTING.md`** — the development rules. TDD rhythm, the three gates, the
   ban on putting logic in YAML.
2. **`ARCHITECTURE.md`** — the code map and the invariants. Its "Architecture
   Invariant" callouts are the contract: breaking one requires amending that
   document in the same commit, with the reason.

Do not infer the architecture from the code. The code is younger than the documents
and is their application, not their source.

## The three gates

Before every commit, all three pass — and CI re-checks them on every PR:

```bash
mise run gates
```

That runs the three in parallel. Individually, or without `mise`:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

Other tasks: `mise tasks` lists them. `mise run commits` validates this branch's
commit messages, `mise run changelog` previews the changelog.

## The TDD rhythm, and the step that gets skipped

1. Write the failing test.
2. **Run it, and check that it fails for the right reason.**
3. Write the minimal implementation.
4. Run it again.
5. Commit.

Step 2 is not a formality. A test that passes before the implementation exists tests
nothing. Checking *why* it fails also catches the case where it merely fails to
compile over a typo. Report the actual failure you saw, not the one you expected.

## Git flow

**Never commit straight to `main`.** It is protected: pull requests are required,
the three checks must be green, history must stay linear, force pushes and deletions
are refused, and the rules apply to administrators too. One branch per task
(`<type>/<kebab-description>`), rebase-and-merge.

**Commit messages are one line, in English, in Conventional Commits format. No
body.**

```
feat(fake): add persistent per-key call counter
```

Types: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `chore`, `ci`, `build`.
Scopes: `fake`, `core`, `cli`, `conformance`. Imperative subject, lowercase, no
trailing period, 72 characters at most. A breaking change is marked with `!`
(`feat(fake)!:`), never with a `BREAKING CHANGE:` footer — there is no body.

The changelog is generated from the commits by `git-cliff`, and messages are
validated in CI by `committed`: **a subject line is a release-notes line**, so write
it for whoever reads those. `committed` catches unknown types and scopes, trailing
periods, non-imperative subjects and over-long ones; it cannot enforce the lowercase
subject, so that one is on you.

The *why* of a change goes in **the PR description**; the *why* of the code goes in
the documentation.

## Reminders that are expensive to forget

- **Everything in this repository is in English.** Identifiers, keywords, doc
  comments, error messages, assertion messages, test names, commit messages,
  documents. No exceptions, no boundary to remember.
- **No comments in files.** Only `//!` and `///` are allowed — no `//` inside a
  function body, no `#` in a configuration file. The *why* moves up: into the item's
  `///`, into a lint attribute's `reason = "…"`, into an assertion message, into the
  PR description, or into `ARCHITECTURE.md` / `CONTRIBUTING.md`. When an explanation
  has no item to attach to, extract a named function whose `///` carries it.
- **A test name describes a behaviour**, not a function:
  `catch_all_responds_and_flags_itself_in_the_journal`, not `test_catch_all`. When a
  test guards a specific pitfall, say so in the assertion message.
- **No logic in the case format.** No conditionals, no loops, no interpolation. Logic
  moves out into an executable, through one of the three hooks.
- **No `unwrap()` and no `expect()`** — these are workspace `deny` lints, not a wish.
  If a case really is impossible:
  `#[expect(clippy::unwrap_used, reason = "…")]` — `expect` rather than `allow`,
  because it also warns once the exemption becomes unnecessary. Tests are exempted by
  a crate-level `cfg_attr`.
- **Every public item is documented** (`missing_docs` + `-D warnings`).
- **No `async`, no `unsafe`, no speculative genericity.** A type parameter is only
  introduced with a second real implementor in sight.
- **`thiserror` in libraries, `anyhow` in binaries.** Context goes in the variant's
  fields, not in a formatted string. Messages start lowercase with no trailing
  period — they chain.
- **Unix only.** No `#[cfg(windows)]`.
- **`gaveldrop-fake` depends on no other crate in the repository.** This is the one
  invariant the compiler will not remind you of.
- **Doc comments on the case format types are user-facing tooltips**: they travel
  through the JSON schema all the way to the editor.

## How to report your work

State what you ran and what it printed. "The tests pass" is worth nothing without the
output. If a step was skipped, say it was skipped. If part of a task is blocked,
finish everything else and say plainly what you left out and why.

Do not claim a behaviour works because the code looks right. Run it.

## What never gets committed

`.gitignore` keeps them out, but worth knowing: `PROJECT-BRIEF.md` and
`docs/superpowers/`. Writing there is fine; it just never leaves the machine.
