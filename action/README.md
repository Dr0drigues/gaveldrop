# The gaveldrop action

Runs your cases in GitHub Actions, with failures annotated on the line of the assertion that broke.

```yaml
- uses: actions/checkout@v4
- uses: Dr0drigues/gaveldrop/action@v0.1.0
```

That is the whole job. It downloads the release archive for the runner's platform, checks the
published checksum, installs the two binaries side by side, puts them on `PATH`, and runs your suite
with `--annotate`.

No Rust toolchain, which is the point: a project whose subject is Node, Python or a shell function has
no reason to have one.

## Inputs

| Input | Default | What it is for |
|---|---|---|
| `version` | `v0.1.0` | The release to install. **Not** `latest`, deliberately — see below. |
| `args` | `--annotate` | Arguments for `gaveldrop`. |
| `working-directory` | `.` | Where `gaveldrop.yaml` lives. |
| `install-only` | `false` | Install and stop, for a workflow that decides its own command. |

## Keeping a report

```yaml
- uses: Dr0drigues/gaveldrop/action@v0.1.0
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
  - uses: Dr0drigues/gaveldrop/action@v0.1.0
    with:
      args: --shard ${{ matrix.shard }}/3 --report-json shard-${{ matrix.shard }}.jsonl
```

Merging the shards is `cat`. See `docs/ci.md`.

## Why the version is pinned rather than `latest`

An action referenced as `@v1` and installing `latest` is one archive-format change away from breaking
every workflow that uses it, at a moment nobody chose. Here the tag that publishes the binaries is the
tag that publishes this file, so an action can only ever install a release it was written against.

Which also means: **this action lives in the gaveldrop repository, not in one of its own.** Keeping it
separate is the usual arrangement and the usual way to get `v1` of an action quietly meeting `v0.3.0`
of the tool.

## Platforms

Linux and macOS, x86_64 and aarch64. A Windows runner fails with a message saying so rather than a
download that 404s — isolation rests on symlinks, on `PATH` and on Unix configuration directories, so
Windows is another project rather than an adjustment.
