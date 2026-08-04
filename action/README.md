# The gaveldrop action

Runs your cases in GitHub Actions, with failures annotated on the line of the assertion that broke.

```yaml
- uses: actions/checkout@v4
- uses: Dr0drigues/gaveldrop/action@v1
```

That is the whole job. It downloads the release archive for the runner's platform, checks the
published checksum, installs the two binaries side by side, puts them on `PATH`, and runs your suite
with `--annotate`.

No Rust toolchain, which is the point: a project whose subject is Node, Python or a shell function has
no reason to have one.

## Inputs

| Input | Default | What it is for |
|---|---|---|
| `version` | the ref's own version | The release to install. Read from the manifest beside the action; override to pin. |
| `args` | `--annotate` | Arguments for `gaveldrop`. |
| `working-directory` | `.` | Where `gaveldrop.yaml` lives. |
| `install-only` | `false` | Install and stop, for a workflow that decides its own command. |

## Keeping a report

```yaml
- uses: Dr0drigues/gaveldrop/action@v1
  with:
    args: --annotate --report-junit junit.xml

- uses: actions/upload-artifact@v4
  if: always()
  with:
    name: cases
    path: junit.xml
```

`if: always()` matters: the report is most useful precisely when the step before it failed.

## Sharding

```yaml
strategy:
  matrix:
    shard: [0, 1, 2]
steps:
  - uses: actions/checkout@v4
  - uses: Dr0drigues/gaveldrop/action@v1
    with:
      args: --shard ${{ matrix.shard }}/3 --report-json shard-${{ matrix.shard }}.jsonl
```

Merging the shards is `cat`. See `docs/ci.md`.

## Which ref to write, and what each one costs

| You write | You get | Choose it when |
|---|---|---|
| `@v1` | the newest release of the 1.x line, and its binaries | you want fixes without touching your workflow |
| `@v0.1.11` | exactly that release, for ever | you want the same bytes on every run |

`v1` is a **moving** tag, the way `actions/checkout@v4` is: it is repointed at each release. That is the
convention every action in the ecosystem is consumed by, and it is the opposite of moving a *version*
tag — `v0.1.11` never moves, and never will.

Either way the action installs the binaries **that came with the ref you named**, because it reads the
version out of the manifest sitting beside it rather than asking for the newest release. That is the
distinction worth keeping: `@v1` follows the line you chose, it does not follow whatever exists.

What it never does is install the newest release regardless of the ref. That would hand an unknown
archive format to a file that predates it — one format change away from breaking every workflow, at a
moment nobody chose.

Which also means: **this action lives in the gaveldrop repository, not in one of its own.** Keeping it
separate is the usual arrangement and the usual way to get `v1` of an action quietly meeting `v0.3.0`
of the tool.

## It is not for a consumer with its own adapter

This action downloads the binaries and runs the `gaveldrop` command, which only knows the built-in
adapters. If you wrote your own, it is compiled into your crate and the binary cannot see it — your
cases are refused, correctly:

```
case `blackboard` would invoke nothing: no adapter recognises it.
```

Nothing here helps you, not even `install-only`: you never run our binary. Your job is `cargo test`,
with the suite driven from a test through `runner::run_all_with`. See the section for that in
`docs/ci.md` — including the one thing that bites, which is that cargo hides a passing test's output.

## It installs gaveldrop, not what your cases need

If a case's subject is a zsh function, the runner needs zsh. If a case runs `node bin/sync.js`, the
runner needs node. The action brings the two gaveldrop binaries and nothing else — a faked dependency
costs you nothing, but the thing under test has to be there.

gaveldrop says which one is missing:

```
FAIL an-unresolved-variable-reaches-the-terminal  0/8
    setup
      expected  the case runs
      got       starting `zsh`: No such file or directory (os error 2)
```

This repository's own CI hits it: macOS ships zsh, Linux does not, so the job that exercises this
action installs it before calling us.

## Platforms

Linux and macOS, x86_64 and aarch64. A Windows runner fails with a message saying so rather than a
download that 404s — isolation rests on symlinks, on `PATH` and on Unix configuration directories, so
Windows is another project rather than an adjustment.
