# gaveldrop

[![gaveldrop](https://dr0drigues.github.io/gaveldrop/badge.svg)](https://dr0drigues.github.io/gaveldrop/)

A test engine where **one case is one YAML file**. A case describes how to invoke a
program, how its dependencies must respond, and what the result must contain.

```yaml
name: sync-refuses-a-dirty-repository
weight: 5
setup:
  run: ["node", "bin/sync.js", "--dry-run"]
fake:
  rules:
    - match: { bin: git, args_contain: "status --porcelain" }
      stdout: " M src/index.js"
    - match: {}
      exit: 127
      stderr: "unexpected call"
expect:
  exit_code: 1
  stderr:
    contains: ["dirty repository"]
  calls:
    git: 1
    gh: 0
```

The subject is a process, so the same file shape works whether `bin/sync.js` is
JavaScript, Python, Java, Kotlin or Rust — only the `run:` line changes. The faked `git`
answers in a directory that need not be a git checkout at all.

The last two lines are often the interesting part: they assert that `gh` was **not**
called. The answer can be right while the side effect was wrong.

## What a failure looks like

```
FAIL k9s-leaves-no-unresolved-variable  0/8
    expect.stdout.absent[0]
      expected  nowhere: "ZSH_ENV"
      got       scriptPath: $ZSH_ENV_DIR/scripts/fmt.zsh
ok   sync-refuses-a-dirty-repository  5/5

gaveldrop — 2 cases · 1 passed · 1 failed · 0 tolerated · score 5/13 · 61ms
```

Which case, which expectation, which value. Without opening gaveldrop's code.

## Running it

Download the archive for your platform from the [releases][releases] and put the two binaries it
holds anywhere on your `PATH`. No Rust toolchain needed, which matters when the subject under test is
Node, Python or a shell function.

[releases]: https://github.com/Dr0drigues/gaveldrop/releases

Or with cargo:

```console
$ cargo install gaveldrop-cli gaveldrop-fake --locked
```

**Two crates, and it is not an oversight.** The fake is an executable because a subject finds a
faked tool by name on `PATH`, and cargo installs the binaries of the crate you name rather than its
dependencies' — so the cli crate has no way to bring it along. Install one and gaveldrop tells you
which command is missing. The archive holds both side by side, which is why it needs no second step.

```yaml
# gaveldrop.yaml
cases: tests/cases/**/*.yaml
fake:
  bins: [git, gh]
```

```console
$ gaveldrop
```

Exit code 0 when nothing failed. `--verbose` prints what the engine decided before each case —
which adapter claimed it, the isolated root, the tools faked or hidden — for a case that does
not do what you expect, which is a different problem from a case that fails.

Editors that speak the YAML language server protocol give completion and validation while you
write a case, from the generated `docs/case.schema.json` — no plugin involved.

Adopting it in an existing project, with the six mistakes everyone makes first and the
message gaveldrop prints for each: `docs/adopting.md`.

Writing your own adapter, in your own crate, and running your suite through it:
`docs/conformance.md`.

## Three properties, in this order

1. **A case is readable and writable by hand** — and therefore generatable by an agent.
   That is what makes coverage cheap.
2. **The project under test changes nothing** to become testable. No instrumentation, no
   test mode in production code.
3. **A failure is diagnosable without reading gaveldrop.**

## Status

Early, and honest about it. Working today: the fake engine, the case format and its
schema, isolation, the process adapter, `exit_code` / `stdout` / `stderr` / `calls`
expectations, the weighted report, and the command-line facade. gaveldrop runs its own
cases through itself.

Working too: `files` expectations, structured events with named invariants — the order things
happened in, exact counts including a `0` that proves something never fired, and rules that hold
across a whole run: `docs/events.md` — the `setup` and `expect` hooks — `docs/hooks.md` — the JSON
Lines and HTML reports, and the conformance kit every adapter must pass, which is also how you
validate an adapter of your own: `docs/conformance.md`.

And the shell, where the subject is a function rather than an executable: sourced from your
repository, invoked with arguments, its dependencies faked the same way a binary's are —
`docs/shell.md`. It was the test of whether the core is generic; `ARCHITECTURE.md` records
what that cost, including the part that was not foreseen.

And the web, where the subject stays alive: started, polled until it answers, interrogated
across several steps, its own upstream APIs faked through a second door onto the same rule
engine — `docs/web.md`.

A case can assert on a value inside a JSON body by path, which is what GraphQL needs since it
answers `200` for a failed operation; and it can name a value from one exchange to use in the
next. What it cannot do is **compute** one — that line is an invariant, and the reasoning is in
`ARCHITECTURE.md`.

And the continuous-integration surface: a failure annotated on the line of the assertion that
broke, JUnit for a dashboard, thresholds in the project's own configuration, and a suite split
across runners where `cat` is the whole merge step — `docs/ci.md`.

And a case can declare the environment its subject reads — `setup.env` for a module guarded by a
flag or a tool locating itself through a directory — and `setup.hide` for the tools that must be
findable nowhere, which is how "warns when the binary is missing" becomes provable rather than a
verdict depending on what the machine has installed.

Published at `0.1.6`: `gaveldrop`, `gaveldrop-fake`, `gaveldrop-cli`, `gaveldrop-conformance`.
Early enough that the version says so.

Not built yet: a prebuilt binary per platform, so a project whose subject is Node or Python does
not need a Rust toolchain, and editor plugins. `ROADMAP.md` tracks it as a checklist, batch by
batch, including the gaps that are accepted rather than overlooked.

Unix only. See `ARCHITECTURE.md` for the design, `ROADMAP.md` for what is coming, and
`CONTRIBUTING.md` for the rules.
