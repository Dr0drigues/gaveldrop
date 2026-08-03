# Running gaveldrop in continuous integration

Everything here works on top of the binary. There is no plugin to install and no service to
configure — a failure annotated on the right line is `--annotate`, and a build that fails when the
suite misses its bar is a `gate:` block.

## The shortest job that works

```yaml
name: Cases
on: [pull_request]

jobs:
  cases:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: Dr0drigues/gaveldrop/action@v0.1.1
```

That is all of it. The action downloads the release archive for the runner's platform, checks the
published checksum, installs the two binaries side by side, and runs your suite with `--annotate`. No
Rust toolchain — which is the point when the subject under test is Node, Python or a shell function.

It lives in the gaveldrop repository rather than one of its own, so the tag that publishes the
binaries is the tag that publishes the action: it cannot install an archive whose format it does not
know. `action/README.md` has its inputs.

The rest of this document is the same thing without the action, which is worth reading if you want to
see what it does or need something it does not offer.

## A job that says what broke, and where

```yaml
name: Cases
on: [pull_request]

jobs:
  cases:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo install gaveldrop-cli gaveldrop-fake --locked
      - name: Run the cases
        run: gaveldrop --annotate --report-junit junit.xml
      - name: Keep the report
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: cases
          path: junit.xml
```

**Two crates, and `gaveldrop-cli` rather than `gaveldrop`.** The executable is called `gaveldrop` and
lives in the `gaveldrop-cli` crate — `cargo install gaveldrop` fails with *there is nothing to
install in `gaveldrop v0.1.1`, because it has no binaries*. And the fake is a **second** executable,
because a subject finds a faked tool by name on `PATH` and cargo installs the binaries of the crate
you name rather than its dependencies'. Install only the first and every case that fakes anything
fails, with a message naming the command you are missing.

`--locked` because a test tool that resolves different dependency versions on different days is a
test tool that fails on different days.

It costs about half a minute to build on a runner, which is why the step is separate — cache it with
`Swatinem/rust-cache@v2` if that matters to you.

**Or skip the toolchain entirely** and download the archive for the runner's platform from the
releases. It holds both binaries side by side, so there is nothing to install and no `PATH` to
arrange, and a runner testing a Node or Python project needs no Rust at all:

```yaml
      - name: Install gaveldrop
        run: |
          curl -fsSL https://github.com/Dr0drigues/gaveldrop/releases/download/v0.1.1/gaveldrop-v0.1.1-x86_64-unknown-linux-musl.tar.gz \
            | tar -xz -C /tmp
          sudo install /tmp/gaveldrop-v0.1.1-*/gaveldrop /tmp/gaveldrop-v0.1.1-*/gaveldrop-fake /usr/local/bin/
```

Two steps rather than piping straight into `/usr/local/bin`, because the archive also carries the
README and the licences and you do not want those on your `PATH`. `install` names the two files
explicitly, so nothing else can arrive by accident.

The Linux archives are statically linked against musl, so they run on a distribution older than the
one that built them.

There is no `gaveldrop/action` yet, so one of these two is the supported way to run it in CI, and
everything either needs is here.

`--annotate` writes one workflow command per failing case, so GitHub shows the failure **on the line
of the assertion that broke** rather than in a log nobody scrolls:

```
::error file=tests/cases/an-order-is-created.yaml,line=10,title=an-order-is-created::…
```

Line 10 is the `contains:` line, not the `expect:` block containing it.

`if: always()` matters: the report is most useful precisely when the step before it failed.

A tolerated failure — a case with `allow_fail: true` — becomes a `::warning` instead. Visible without
failing the check, which is what declaring it asked for.

## Failing the build on the project's own terms

Every case passing is one question. Whether the suite met its bar is another, and it belongs in
`gaveldrop.yaml` rather than in a command line — a bar that moves depending on who typed the command
is not a bar:

```yaml
cases: tests/cases/**/*.yaml
gate:
  min_score: 80
  max_tolerated: 2
  fail_above_weight: 8
```

- `min_score` — the least weighted score that counts as a pass. The question a project with a long
  tail of low-weight cases actually cares about.

  **It is a total, not a percentage.** The `80` above is 80 points, so it only makes sense for a suite
  whose weights add up to more than that — copy it into a smaller suite and every run fails. A
  threshold above the suite's own total now says so instead of reporting a shortfall:

  ```
  gaveldrop: gate.min_score is 80 and the whole suite is worth 68, so this threshold can never
  be met. It is a weighted total, not a percentage: add up the `weight:` of your cases to
  choose it
  ```
- `max_tolerated` — how many `allow_fail` cases may fail before the exemption is a lie. An exemption
  nobody counts becomes a habit.
- `fail_above_weight` — a weight above which one failing case fails the run alone. Ninety percent of
  the weight holding is no comfort when the case that broke is the one that mattered. A *tolerated*
  failure does not trip this; that is what `max_tolerated` is for.

Every unmet threshold is reported, not just the first — fixing one to discover another is two runs
where one would do. The binary exits non-zero for a missed gate exactly as it does for a failing case:
a caller asking "did this pass" wants one answer.

An absent `gate:` block enforces nothing.

## Splitting the suite across runners

```yaml
jobs:
  cases:
    strategy:
      matrix:
        shard: [0, 1, 2]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo install gaveldrop-cli gaveldrop-fake --locked
      - run: gaveldrop --shard ${{ matrix.shard }}/3 --report-json shard-${{ matrix.shard }}.jsonl
      - uses: actions/upload-artifact@v4
        with:
          name: shard-${{ matrix.shard }}
          path: shard-${{ matrix.shard }}.jsonl

  merge:
    needs: cases
    runs-on: ubuntu-latest
    steps:
      - uses: actions/download-artifact@v4
      - run: cat shard-*/shard-*.jsonl > all.jsonl
```

`cat` is the whole merge step. The report is a list of outcomes with the summary computed from it,
never a frozen summary, so concatenation is a merge — and no shard writes a summary line that would be
counted as a case.

Shards are `index modulo count`, **interleaved rather than contiguous**: cases that sort together
usually share a subject, so contiguous blocks would put all the slow ones on one runner. Discovery is
sorted, so every machine computes the same partition without a coordinator.

`--shard 4/3` is refused rather than run as an empty suite, because an empty run reports success and
that is the worst possible answer to a typo in a matrix.

## Running one case while you work

```sh
gaveldrop --only an-order          # every case whose path contains this
```

It matches the **path**, so naming a file after its case — the convention in this repository — makes
the name you read in a failure the fragment you type. A fragment matching nothing is an error, for the
same reason as a bad shard.

## Faking in CI what you pass through on a laptop

A case with `exec: real` reaches the real tool. That is right on a machine with credentials and a
network, and impossible in CI:

```yaml
fake:
  bins: [gh]
  no_passthrough: true
```

Usually set in the CI job rather than committed — write a `gaveldrop.ci.yaml` and pass
`--config gaveldrop.ci.yaml`, so a laptop keeps reaching the real tool.

Each rule that passes through must then declare what it answers instead:

```yaml
fake:
  rules:
    - match: { bin: gh, args_contain: "pr view" }
      exec: real
      stdout: '{"state":"OPEN","title":"a faked pull request"}'
    - match: {}
      exit: 127
```

Without that `stdout`, the run is **refused** and names the rule. Substituting an empty response would
make the subject see silence where it expected the real tool's output — a wrong answer dressed as a
right one.

## The reports, and who reads them

| Flag | Format | Read by |
|---|---|---|
| *(default)* | terminal, streamed per case | you, while it runs |
| `--annotate` | GitHub workflow commands | the pull request, on the failing line |
| `--report-junit` | JUnit XML | a CI dashboard |
| `--report-json` | JSON Lines, one outcome per line | merging shards, and anything you script |
| `--report-html` | one self-contained page | someone you send a link to |

They compose: a real job usually wants the terminal, annotations and JUnit at once, which is what the
first example does.

JUnit is the only one that cannot stream — its header carries the totals, so it is written when the
last case finishes. The others emit as they go.
