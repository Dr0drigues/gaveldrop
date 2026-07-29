# gaveldrop

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

gaveldrop — 2 cases · 1 passed · 1 failed · 0 tolerated · score 5/13
```

Which case, which expectation, which value. Without opening gaveldrop's code.

## Running it

```yaml
# gaveldrop.yaml
cases: tests/cases/**/*.yaml
fake:
  bins: [git, gh]
```

```console
$ gaveldrop
```

Exit code 0 when nothing failed. Editors that speak the YAML language server protocol
give completion and validation while you write a case, from the generated
`docs/case.schema.json` — no plugin involved.

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

Working too: `files` expectations, structured events with named invariants, the `setup`
and `expect` hooks — see `docs/hooks.md` — the JSON Lines and HTML reports, and the
conformance kit every adapter must pass, which is also how you validate an adapter of your
own: `docs/conformance.md`.

And the shell, where the subject is a function rather than an executable: sourced from your
repository, invoked with arguments, its dependencies faked the same way a binary's are —
`docs/shell.md`. It was the test of whether the core is generic; `ARCHITECTURE.md` records
what that cost, including the part that was not foreseen.

Not built yet: the web adapter. `ROADMAP.md` tracks it as a checklist, batch by batch,
including the gaps that are accepted rather than overlooked.

Unix only. See `ARCHITECTURE.md` for the design, `ROADMAP.md` for what is coming, and
`CONTRIBUTING.md` for the rules.
