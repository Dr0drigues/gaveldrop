# Hooks

Three places a project supplies an executable. The unit of extension is **an executable**, not
a Rust crate: a Kotlin, Python or shell project hooks in exactly what a Rust project does. Had
the extension point been a trait, only Rust could extend gaveldrop.

The contract is **this protocol**, not the convenience packages that may be published per
ecosystem later. A language with no package works with three lines of `jq`.

| Hook | Declared in | Receives on stdin | Answers on stdout |
|---|---|---|---|
| `setup.exec` | a case | the `setup` block as JSON, without `exec` | nothing; the exit code is the verdict |
| `expect.exec` | a case | the observations as JSON | `{"ok": bool, "diffs": [...]}` |
| `fake.render` | a scenario | the selected rule and the call | the bytes the fake must emit |

## Two directories, and both matter

A hook is **resolved** against the project root, because that is what a case author means when
they write `./tests/hooks/foo.sh`. It **runs** with its working directory set to the isolated
root, because that is what it prepares or inspects.

So a hook that writes into `$HOME` writes into the case's home, never yours. Every variable
isolation defines is set, and everything the project listed under `clear_env` is removed.

Standard input is closed after the payload is written, so a hook may read to end-of-input
without hanging.

## `setup.exec`

Runs **before** the tree snapshot: what a hook prepares is scenery, not the subject's work, so a
`files` assertion never reports the fixtures it laid down.

```json
{ "pattern": "ring", "agents": ["alpha", "bravo"] }
```

Everything in `setup` except `exec`, which is the hook's own path and no part of what it is
being told to prepare. `run` is included when present.

A non-zero exit fails the case, and the hook's standard error is quoted in the report:

```
the setup hook './prepare.sh' exited with 4: cannot render the template
```

## `expect.exec`

Receives the whole `Observations` object — `exit`, `stdout`, `stderr`, `calls`, `events`,
`files`, `ext`. It exists to check what the core cannot, so nothing is withheld.

```json
{ "ok": false, "diffs": [{ "path": "expect.exec.rows", "expected": "3", "got": "0" }] }
```

`path` is what the report shows and what pull-request annotation will one day resolve to a line,
so name it after what you checked.

**`ok: false` with an empty `diffs` still fails the case.** The core supplies a placeholder
rather than letting a terse refusal pass, because otherwise `ok: false` would mean nothing.

An answer that is not valid JSON is a protocol error naming the hook, not a silent pass.

## `fake.render`

See `ARCHITECTURE.md`, `crates/gaveldrop-fake`. It shapes bytes; it does **not** decide whether
the faked tool succeeded. The rule's `exit` stands, and a non-zero exit from the hook is a
harness failure rather than a simulated one.

## A worked example

`tests/hooks/count-lines.sh`, used by
`tests/cases/a-project-hook-can-check-what-the-core-cannot.yaml`:

```sh
#!/bin/sh
lines=$(jq -r '.stdout' | grep -c .)
if [ "$lines" -eq 3 ]; then
  printf '{"ok":true,"diffs":[]}'
else
  printf '{"ok":false,"diffs":[{"path":"expect.exec.lines","expected":"3","got":"%s"}]}' "$lines"
fi
```

```yaml
name: a-project-hook-can-check-what-the-core-cannot
weight: 5
setup:
  run: ["sh", "-c", "printf 'one\ntwo\nthree\n'"]
expect:
  exit_code: 0
  exec: ./tests/hooks/count-lines.sh
```

It uses `jq`, which is present on the GitHub runner images. A hook needs no particular tool —
this one reads JSON, so `jq` is the shortest honest way to show it.
