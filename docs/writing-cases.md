# Writing a suite that keeps working

`docs/adopting.md` shows how to get a first case running. This page is about the habits that decide
whether a suite is still trusted a year later — and every one of them is here because something went
wrong without it, in this repository or in one of the two projects that use it.

## The shape on disk

```
gaveldrop.yaml                    ← the project: where cases are, what is faked, the event vocabulary
tests/cases/
├── deploy/
│   ├── refuses-a-dirty-tree.yaml
│   └── tags-the-release.yaml
└── sync/
    └── retries-once-then-gives-up.yaml
```

**One case, one file, named after the case it holds.** Three mechanisms depend on it, so it is not a
matter of taste:

- `--only` matches the **path**, so a file named after its case makes the name you read in a failure the
  fragment you type;
- an editor plugin navigates from a tree node to the file by that name;
- two cases may not share a name, and the run is refused before anything is prepared if they do.

**Group with directories, not with an extra file.** The `cases` pattern is a glob — `tests/cases/**/*.yaml`
already walks subdirectories. There is deliberately no `suite.yaml` that cases inherit from: a case that
needed a second file read to be understood would stop being reviewable in a pull request and generatable
by an agent, which are the two properties the whole format is built for. What is genuinely shared —
the faked binaries, the event vocabulary, the named invariants, the gate — is already in
`gaveldrop.yaml`, once.

## Naming

**Name the case after what it proves, not after what it does.** `deploy-refuses-a-dirty-tree` beats
`test-deploy-4`: the name is the identifier in every report this project writes, and it is what someone
reads six months later with no context.

A name has to contain something visible. An empty one, whitespace, and a zero-width space are all
refused — the last one because it produced `<testcase name="">` in a JUnit file, valid XML that no reader
can act on.

**`weight` is how much the case matters**, not how much work it was. It orders failures in a report and
feeds the gate. A smoke test that everything depends on is heavy; an edge case is light.

**`allow_fail` is a claim, and it is never made by omission.** A known defect you have decided to live
with is worth writing down; a case that silently stopped mattering is not.

## Asserting

**Say what you care about and nothing else.** An omitted expectation is not checked, a listed event is a
subsequence rather than an exact list, and each event entry matches a subset of one event's fields. That
is what keeps a case from breaking the day the subject gains one log line — and a case that breaks for
that reason gets deleted rather than maintained.

**Assert the numbers that regress.** A consumer ran mutation testing against their own suite: eight
injected defects, five caught. The three that slipped were all in numbers no case asserted — token counts
corrupted, a cost doubled, an agent count wrong. Their events carried those fields and nothing looked at
them.

**Do not assert a computed float.** `tokens × rate` can produce `0.30000000000000004` where your case
says `0.3`, and the case is then red one run in two for a reason that is not a bug. Assert the integers
that feed it, and `cost: 0.0` where it must be nothing — zero is exact either way.

**`event_counts` with a `0` is the only way to prove something never happened.** That the retry did not
fire, that the budget warning was not emitted. A subsequence says what did occur; `stdout.absent` matches
text rather than structure. It needs the `events:` block declared, or it passes while proving nothing.

**There is no key for how long anything took, and there will not be.** A case that failed because a
machine was loaded lies one run in two. Durations are reported everywhere and asserted nowhere.

**Say which files must be left alone, not just which must be written.** `not_written` names paths the
subject must not create, modify or remove, where `no_new_files: true` says it wrote nothing at all. Use
the blanket one for a tool that claims to only read, and the list for the ordinary case — a subject that
writes its own log and must not touch your configuration. A file assertion is the one that catches the
defect that appears in no output, which is what this project was scoped around.

**When two things belong together, say so with `line_includes`.** `contains` proves a fragment exists
somewhere and never that two of them are on the same row — so a case asserting
`contains: ["KUBE", "DOCKER", "active", "inactive"]` against a `MODULE / STATUS` table passes on that
table *and* on the same table with every status swapped. A consumer found it by swapping them on purpose.

```yaml
    line_includes:
      - ["KUBE", "active"]
```

Each group holds when one single line carries all of its values as **words**. Words matter here rather
than being a nicety: `inactive` contains `active`, so a substring comparison would hold on the very row
the case exists to catch.

And do not reach for the alternative that spacing gives you — `contains: ["KUBE         active"]` works
and makes changing `{:<12}` to `{:<14}` fail a test about behaviour, which is the case that gets deleted
rather than maintained.

**Say why a case asserts less than its neighbour.** Sometimes it has to: the subject cannot produce a
field on that path, an exchange answers before the value exists, a platform does not report it. Write the
reason in a comment where the assertion would have been.

This is the one place the rule above is not enough. "Say what you care about and nothing else" makes a
gap invisible, and a reviewer then reads a deliberate weakening as an oversight — or, worse, aligns the
*strong* neighbour down to match, on the reasonable assumption that the two cases should agree. A missing
assertion nobody explained is indistinguishable from one nobody thought of, which is the same reason
`allow_fail` is a claim and never made by omission.

*This one comes from the project that runs the largest suite on gaveldrop, and it is their habit rather
than ours.*

## Exchanges

**A `step` is a second exchange with the subject, not a section heading.** Add one when the subject really
is invoked twice — a command that writes state and a second that reads it back, a service answered across
three requests, a run followed by a replay. Do not add one to make a report prettier: the tree follows the
tests, not the other way round.

**Name your steps.** The name is what a failure says instead of an index, and what a test tree shows
instead of `step 2`. `steps[1] "the replay reads the same run".events[0]` locates a problem; `steps[1]`
makes someone count.

Each exchange is checked against what *it* produced — its own streams, its own events, its own file
effects — and the case's own `expect:` is checked against everything the run produced. Both are true at
once and neither is a substitute for the other.

**The exchanges of one case share one isolation.** The same root, the same `HOME`, the same faked tools,
the same journal. That is what makes write-then-read work at all: the second exchange sees the files the
first one wrote.

```yaml
steps:
  - name: writes state
    request: { run: ["./my-tool", "sync"] }
    expect: { exit_code: 0 }
  - name: reads it back
    request: { run: ["./my-tool", "status"] }
    expect:
      stdout: { contains: ["1 change pending"] }
```

Load-bearing and easy to miss, so it is worth saying rather than implying: a case that needed two
isolations is two cases.

**A *value* crosses an exchange through `capture:`**, not through the filesystem — an identifier the
subject answered with, substituted into the next request as `$name`:

```yaml
steps:
  - name: creates an order
    request: { method: POST, path: /orders }
    capture: { order_id: data.id }
  - name: reads it back
    request: { path: /orders/$order_id }
```

**Honoured by the web adapter, and by an adapter that implements it.** The process and shell adapters
report every `capture:` a case declares as *missed* — deliberately: a process answers text, and deciding
that its output is a JSON document to walk by path would invent a meaning for the format rather than
implement one. Reported rather than ignored, so a case that declares one is told at `capture.<name>`
instead of failing an exchange later on a name that silently stayed literal.

## Before you trust a case

**Make it fail on purpose, once.** Change the expected value, run it, watch it go red, change it back.
This is the only rule here with no exceptions: a case that cannot fail proves nothing, and it will sit in
the suite for years looking like coverage.

The failure modes worth knowing before you meet them:

- an assertion that can never hold — `contains: ["2"]` on a count of `12` passes on the wrong answer, so
  use `equals` for a value;
- a path that leaves the isolated root — refused, because nothing out there is observed;
- a case derived from a run you already observed — it passes by construction. Write what should be true,
  then check the subject agrees, rather than recording what the subject said and calling it a
  requirement.

## Where the engine helps

- `--verbose` prints what the engine decided before each case runs: which adapter claimed it, where the
  isolated root is, which tools are faked or hidden, what the declared variables resolved to.
- The HTML report folds each case open on what the subject actually did, which is where you find what you
  should have been asserting.
- `unmentioned files` on a failing case lists what the subject wrote that the case says nothing about.
  It is offered, never counted as a failure.
- `--report-teamcity`, or `GAVELDROP_REPORT_TEAMCITY=1`, draws the suite as a test tree in a JetBrains
  IDE and in TeamCity.
