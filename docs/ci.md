# Running gaveldrop in continuous integration

Everything here works on top of the binary. There is no plugin to install and no service to
configure — a failure annotated on the right line is `--annotate`, and a build that fails when the
suite misses its bar is a `gate:` block.

## Which of the two you are

Everything below assumes the `gaveldrop` **binary** runs your cases. That holds if your subject is a
process, a shell function or a service — the adapters are built in, so the binary recognises your cases.

It does **not** hold if you wrote your own adapter. A custom adapter is compiled into your crate, and
the binary cannot reach it: a case carrying your own vocabulary is refused, correctly and loudly.

```
FAIL blackboard  0/8
    got  case `blackboard` would invoke nothing: no adapter recognises it.
         Add `run: [...]` ... setup holds agents, flags, input, pattern, scenario
```

If that is you, skip to **[CI with your own adapter](#ci-with-your-own-adapter)**. The action and the
`gaveldrop` command have no role there, not even `install-only` — you never run our binary.

## The shortest job that works

```yaml
name: Cases
on: [pull_request]

jobs:
  cases:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: Dr0drigues/gaveldrop/action@v1
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
install in `gaveldrop v0.1.2`, because it has no binaries*. And the fake is a **second** executable,
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
          curl -fsSL https://github.com/Dr0drigues/gaveldrop/releases/download/v0.1.2/gaveldrop-v0.1.2-x86_64-unknown-linux-musl.tar.gz \
            | tar -xz -C /tmp
          sudo install /tmp/gaveldrop-v0.1.2-*/gaveldrop /tmp/gaveldrop-v0.1.2-*/gaveldrop-fake /usr/local/bin/
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

## CI with your own adapter

Your adapter is Rust compiled into your crate, so the job is `cargo test` and not our binary. That is
not a workaround: an adapter is code, and code has to be built by whoever owns it.

```yaml
      - uses: actions/checkout@v4
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace
```

You need no toolchain-free install, no archive and no action — you already have a toolchain, since you
are compiling an adapter with it.

The suite runs from a test through `runner::run_all_with`, with your adapter ahead of the built-ins so
your cases reach it and any plain `run:` case still works:

```rust
#[test]
fn the_suite_passes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = gaveldrop::Config::load(&root.join("gaveldrop.yaml")).unwrap();

    let mut chain: Vec<Box<dyn Adapter>> = vec![Box::new(MyAdapter)];
    chain.extend(gaveldrop::adapters::registry());

    let mut sink = gaveldrop::Tee::new();
    sink.add(Box::new(Terminal::plain(std::io::stdout())));
    sink.add(Box::new(Junit::new(File::create("junit.xml").unwrap())));
    sink.add(Box::new(Badge::new(File::create("badge.svg").unwrap())));

    let report = gaveldrop::runner::run_all_with(
        &config, root, &fake_binary, &mut sink, None, None, &chain,
    )
    .unwrap();

    assert!(report.is_success(), "{} case(s) failed", report.summary().failed);
}
```

Every renderer is available to you — `Tee` fans out to as many as you like — so the reports are the same
files the binary would have written. `docs/conformance.md` covers writing the adapter itself.

### The one thing that bites: cargo captures your output

`cargo test` hides a passing test's standard output and shows a failing one's. Measured, not assumed:

| The test | `::error::` on stdout |
|---|---|
| fails | appears in the log |
| passes | **captured, never seen** |

So annotations for failing cases happen to work — the test fails when they exist — while the
`::warning::` lines a *tolerated* failure produces vanish, because the test passes. A tolerated failure
you cannot see is the exemption becoming a hiding place, which is what `allow_fail` was designed against.

Write them to a file and print it, which is deterministic in both cases:

```rust
sink.add(Box::new(Annotate::new(File::create("annotations.txt").unwrap(), &discovered)));
```

```yaml
      - run: cargo test --workspace
      - name: Annotate the pull request
        if: always()
        run: cat annotations.txt
```

`if: always()` matters for the same reason it does everywhere else here: the step is most useful exactly
when the one before it failed.

The Pages recipe above works unchanged for you — it publishes files, and it does not care which program
wrote them.

## The reports, and who reads them

| Flag | Format | Read by |
|---|---|---|
| *(default)* | terminal, streamed per case | you, while it runs |
| `--annotate` | GitHub workflow commands | the pull request, on the failing line |
| `--report-junit` | JUnit XML | a CI dashboard |
| `--report-json` | JSON Lines, one outcome per line | merging shards, and anything you script |
| `--report-html` | one self-contained page, each case foldable | someone you send a link to |
| `--report-badge` | one SVG, the weighted score | a README |

### What each case did

In the HTML report every case folds open on **what the subject produced** — its exit code, both
streams, which tools it called and how often, the files it wrote. That is the question a verdict does
not answer: a case can pass and still have done something you did not expect, and until now the only
way to look was to write a throwaway case and read `--verbose`.

The verdict stays outside the fold. A failure is readable without a click, because folding it away to
tidy the page would trade the one thing a report exists for.

`<details>` is native HTML, so the page still contains no JavaScript and still opens from a CI artefact
with no network. Streams are cut past a few thousand characters, with the full length named — a subject
writing fifty thousand lines must not produce a page nobody can open.

### The badge

`--report-badge badge.svg` writes the weighted score, coloured by the verdict: green when everything
held, amber when a tolerated failure did fail, red otherwise. Three states rather than two, because a
tolerated failure must look like neither — that is what declaring `allow_fail` asked for.

**It is a photograph of one run, and it says so.** The `<title>` reads *"as of the run that wrote this
file"*, with the counts a label has no room for. A badge implying a live reading without one would be
exactly the kind of green this project exists to refuse, so committing a stale one is on you — publish
it from the job that produced it, or not at all.

```yaml
      - run: gaveldrop --annotate --report-badge badge.svg
      - uses: actions/upload-artifact@v4
        if: always()
        with:
          name: badge
          path: badge.svg
```

No service is involved and none is needed: it is a file, and where it goes is your project's business.

### A link in your README that never changes

The awkward part of publishing either the badge or the page is that a run's artifact has an URL
carrying the run's id, so it moves every time — and `raw.githubusercontent.com` serves everything as
`text/plain`, so a committed HTML file does not render and a committed SVG only works inside a GitHub
README, through its image proxy.

**GitHub Pages solves both**: it serves each file with its real type, at an address that never moves.
One job, on `main` only, and the link in your README is written once:

```yaml
  pages:
    if: github.ref == 'refs/heads/main'
    needs: gates
    runs-on: ubuntu-latest
    permissions: { pages: write, id-token: write }
    environment: { name: github-pages }
    steps:
      - uses: actions/checkout@v4
      - run: |
          mkdir -p site
          gaveldrop --report-html site/index.html --report-badge site/badge.svg || true
      - uses: actions/configure-pages@v5
      - uses: actions/upload-pages-artifact@v3
        with: { path: site }
      - uses: actions/deploy-pages@v4
```

```markdown
[![gaveldrop](https://YOU.github.io/YOUR-REPO/badge.svg)](https://YOU.github.io/YOUR-REPO/)
```

**`|| true` is deliberate and it is the whole design of this job.** It publishes; it does not judge.
Your gate job is what fails a push, and a report is most worth reading precisely when something broke
— so a failing suite has to reach the page anyway. Nothing is hidden by it: the badge carries the
verdict in its colour.

Restrict it to `main`. A pull request deploying over the page would make the README's link show
whatever branch ran last, which is worse than showing nothing.
If you want a badge that merely says a suite exists, `docs/badge.svg` is static and needs no run at all
— see `docs/adopting.md`.

They compose: a real job usually wants the terminal, annotations and JUnit at once, which is what the
first example does.

JUnit is the only one that cannot stream — its header carries the totals, so it is written when the
last case finishes. The others emit as they go.
