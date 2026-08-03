# gaveldrop architecture

> **Status:** the fake engine, the core, the command-line facade, the conformance kit
> and three adapters — process, shell and web — are implemented, with tests passing on
> Linux and macOS. gaveldrop runs its own cases through itself. See "What does not
> exist yet" for what remains.
> This document stands alone: this is where the decisions and the invariants live, and
> nothing else needs opening to understand them.

The case format first existed, and proved itself, in a prototype welded to a single
project: **armadai**, an agent orchestrator written in Rust — nine cases, fifteen
hundred lines of harness. gaveldrop is its extraction and generalisation, and
armadai is its first consumer. References to "the prototype" and to "armadai" in
this document mean that code.

This document is for someone about to modify gaveldrop. It says **where things are
and why they are there**, not what the functions are called.

## Bird's Eye View

gaveldrop runs tests where **one case is one YAML file**. A case describes how to
invoke a program, how its dependencies must respond, and what the result must
contain. gaveldrop prepares an isolated environment, invokes, observes, then
returns a verdict.

Three properties drive every decision below, in this order:

1. **A case is readable and writable by hand** — and therefore generatable by an
   agent. That is what makes coverage cheap, and it is the first thing to degrade
   when attention slips.
2. **The project under test changes nothing** to become testable. No instrumentation
   code, no test mode in production code.
3. **A failure is diagnosable without reading gaveldrop.** The report says which
   case, which expectation, which value it got.

**Architecture Invariant:** the core knows no language, no framework, no tool. It
knows processes, files and lines of text. All knowledge of a particular technology
lives in an adapter or in an executable supplied by the project.

**Architecture Invariant:** a case file holds only **facts** — a path, an expected
string, an exit code, a file's contents. The moment something has to be *decided*,
the logic leaves the YAML and moves into an executable. This is the dam against
drifting into a failed programming language written in YAML. A YAML that grows
conditionals and loops ages badly; a YAML that calls a script ages like the script.

**Architecture Invariant:** everything crossing an extension boundary is
JSON-serialisable data. Never a file handle, never a function, never a live object.
We do not exploit this property today — extensions are executables, and a Rust
consumer goes through the library — but we do not foreclose it either.

**Architecture Invariant:** Unix only. Isolation rests on symlinks, on `PATH`, and
on Unix-style configuration directories. Windows is not an adjustment, it is another
project.

### The placement rule

Without an explicit rule, the boundary between core and extensions becomes a
negotiation at every new observation. So it is fixed once:

> **The core carries everything observable of any process** — exit code, standard
> output and error, files written, outgoing calls.
> **An extension is reserved for what the technology alone can produce** —
> typically internal metrics, or state nothing outside can see.

Direct consequence, already settled: the "events" of a program that emits JSON
lines on its standard output are **observable of any process**. They therefore
belong in the core, not in an extension.

## Code Map

```
crates/
├── gaveldrop-fake/          the fake engine: library plus binary
├── gaveldrop/               the core: case, iso, adapters, verdict, report
├── gaveldrop-cli/           the command-line facade
└── gaveldrop-conformance/   the conformance kit
```

Dependencies flow `gaveldrop-cli → gaveldrop → gaveldrop-fake`. The most reusable
crate sits at the bottom, and that is deliberate.

### `crates/gaveldrop-fake`

The fake engine: deciding which rule applies to a call, keeping a counter,
journaling. It is **both a library and a binary**.

The binary is a thirty-line `main()` on top of the library. It is symlinked under
the name of each binary to fake and placed first on `PATH`.

**API Boundary.** A Rust project that needs particular response rendering builds
*its own* fake binary from this library, with its own renderer. That is the path
armadai takes to emit Claude Code's wire format.

**Architecture Invariant:** `gaveldrop-fake` depends on no other crate in the
repository. If it ends up depending on the core, a consumer that only wants the
engine has to pull in the evaluation, the reports and the case format.

**Architecture Invariant:** a scenario with no catch-all — a rule whose `match` is
empty — is a **load error**, not a tolerated defect. The catch-all is what turns
"an unexpected call happened" into a loud failure instead of silence. It is the
property that makes a case prove anything at all.

**Architecture Invariant:** the two doors share the engine. A dependency faked as an
executable on `PATH` and one faked as a service on a port go through the same
matching, the same per-key counter and the same journal — a project writes
`fake.rules` once. Only the door changes. If the two ever needed different rules, the
engine would be wrong, and a case would stop meaning one thing.

The binary door is an executable out of **necessity**: a subject finds a faked tool by
name on `PATH`, and only an executable is findable that way. A faked service has no
such constraint since gaveldrop starts it, so the HTTP door is a thread using the
engine as a library — no second binary to ship, no start-up handshake. Extensibility
by executable is kept where it earns its cost: `fake.render` is still a hook.

The HTTP door honours the static mode and `render`. One hook protocol, two consumers: the
binary door lets the hook inherit its own streams, since the fake *is* the process the
subject invoked, while the HTTP door captures those bytes and makes them the body. A
project writes the same `fake.render` executable and it works at either door.

`exec: real` has no meaning at the HTTP door — there is no next service along a port — and
is refused **at start-up**, naming the mode: a scenario that cannot work should say so
before the subject is running.

**Architecture Invariant:** the rule's outcome survives the hook, at both doors. The binary
door keeps the rule's `exit` and the HTTP door keeps its `status`; a hook shapes bytes and
never decides. Letting one change the outcome would turn a deliberately shaped failure into
a silent success, which is the single thing a fake must not do. A hook that cannot run is
itself a failure — 125 at the binary door, 500 at the HTTP one — reported rather than
swallowed.

**Architecture Invariant:** journaling is unconditional. A call is journaled even
when the rule passes through to the real binary, even when the catch-all answered,
even when the fake exits in error. The journal is the only source of truth about
*who called what*, and a journal with holes is worse than no journal.

**Architecture Invariant:** the journal is an **append-only file**, never a pipe or
a socket. Each intercepted call is a separate process, and the subject under test
may spawn several in parallel. A file opened `O_APPEND` accepts concurrent writes
with no coordination at all, as long as they stay under a pipe's size — which a
JSON line of this size guarantees by a wide margin.

**Architecture Invariant:** passthrough must never be able to find the fake itself. The
skip is by **file identity**, canonicalising both sides — never by directory. This is
not pedantry: `std::env::current_exe` resolves symlinks on Linux (`/proc/self/exe`) but
not on macOS (`_NSGetExecutablePath` returns the path as invoked), so a directory
comparison passes on one platform and, on the other, the fake finds itself and recurses
until `fork` gives out. Found by CI, not by review — which is why the test matrix covers
both platforms.

**Architecture Invariant:** the call counter's key is supplied by the caller, not
inferred. By default it is the name of the faked binary; armadai puts an agent
identifier there, extracted from the prompt. Two different semantics, one mechanism,
and the choice stays visible in the caller's code rather than hidden in the engine.

Core `match` criteria: `bin`, `args_contain`, `stdin_contains`, `call: N`, and the
catch-all (`match: {}`). A project adds its own by serde composition
(`#[serde(flatten)]`) on its own type, with the engine none the wiser.

Response modes, all four present from day one:

| Mode | What it does | Why it exists |
|---|---|---|
| static | writes the output spelled out in the rule | the common case |
| `exec: real` | passes through to the real binary | `jq`, `sops`, `age` are deterministic and local; what we want from them is the journal entry, not an invented answer |
| `exec: <script>` | delegates to a project executable | the escape hatch for stateful logic |
| `render: <script>` | dresses the response | when the dependency speaks a wire format YAML will never guess |

The fourth mode was added during scoping: the brief assumed "only the response
varies", but a tool that answers with a JSON envelope carrying token counters fits
none of the other three. A tool with no escape hatch gets worked around; that is
why all four ship on day one.

### `crates/gaveldrop` — `case`

The case format, its loading, and the JSON schema that describes it.

**Architecture Invariant:** the JSON schema is **derived from the type**, never
hand-written. It is committed to the repository and regenerated by a test that
fails if the committed file has drifted. That is what makes the format safe to write
by hand and to generate with an agent: a hand-written schema would lie at the first
change of shape.

**Architecture Invariant:** an invalid case fails **at load time**, naming the
offending key. Never three steps later, with a message about something else. The
cost of a bad error message here is paid by every case never written.

**Architecture Invariant:** the core understands exactly three keys of the `setup`
block — `run`, `exec` and `env`. **Everything else in it is opaque** and goes into the
hook untouched. That is what lets armadai write `pattern: ring, agents: […]` without
gaveldrop knowing what a pattern or an agent is, and without the core gaining an
ounce of domain vocabulary.

It was two for the first seven lots, and `env` was added rather than slipped in. The
placement rule decides it: an extension holds what **one** technology can produce, and
every process has an environment — a module guarded by a flag, a tool locating itself
through a directory, and the same subject in Node, Python or zsh. So the core is where
it belongs, beside `clear_env:` which already lives there and removes.

Without it a whole class of subject could not be invoked at all. zanvil is the case
that showed it: every module is guarded by `ZANVIL_MODULE_<NAME>=true` and resolves its
files through `$ZANVIL_DIR`, so no case could load one — and the workaround would have
been to change zanvil so it reads its configuration differently, which is property 2
traded away for a missing key.

**Architecture Invariant:** `PATH` inside the isolation is the fake's symlink directory
followed by the inherited one, and a case may **subtract** from the inherited part with
`setup.hide`. It may never add to it.

Inheriting is not laziness: isolation has to leave `sh`, `printf` and the interpreters
working, and enumerating them would be a list that is wrong on the next machine. But it is
the last place the runner's machine reaches into a case, and it showed: a case asserting
"warns when the tool is missing" passed on a bare runner and failed on a laptop where the
tool was installed. Faking cannot express absence — a fake is a symlink, so it makes the
tool *present*.

Subtraction is directory-wise because that is the only granularity `PATH` has. A shell
walks the directories and reports the first hit; there is no "this directory except one
name". So hiding a tool takes its neighbours with it, which is documented rather than
worked around, and a case that needed one of them fails with a command not found. Loud was
the requirement.

`hide` also withholds the symlink for a tool the project's `fake.bins` lists, rather than
refusing the pair. The first version refused it, on the reasoning that a faked tool is
present by construction so the two declarations could not both be meant. They can:
`fake.bins` is about the suite and `hide` about one case, and the more specific decides.
The refusal made a module with two branches — present, absent — untestable in a single
configuration, because `fake.bins` lives in the configuration and `cases:` takes one
pattern. zanvil paid for that in two configuration files, two case directories and two CI
invocations before this was fixed: a structure imposed by the tool rather than by the
subject.

The `PATH` is composed with one `join_paths` over the kept directories rather than
concatenated with a separator, because concatenation produces an **empty entry** when
nothing survives the filter — and an empty entry means the current directory to a shell,
which here is the isolated root the subject is writing into. Found by a test that expected
an empty result, not by review.

**Architecture Invariant:** the variables a case declares are folded into the isolation's
own list, so every adapter applies them without knowing they exist. Two refusals, both
loud: a name isolation defines cannot be redefined — a case that could point `HOME` back
at the real one would undo the isolation it runs in — and a name `clear_env:` asks to
remove cannot be set, because an adapter clears *after* it sets and the value would
vanish without a word.

Values are expanded strictly: `$GAVELDROP_PROJECT` resolves, an unknown name is an error.
Neither of the other two interpolations would do — `substitute` confines its result under
the isolated home, and `expand_known` is lenient because a *command line* is read by a
shell whose syntax is not ours. An environment value reaches `Command::env` and no shell
ever sees it, so a stray `$TYPO` can only set the variable to something quietly wrong.

**Architecture Invariant:** `setup` is the only open block. Under `fake:` an unknown key
is **refused at load time**, by the loader against the key lists `Scenario`, `Rule` and
`Match` publish, and by the schema in the editor — the two are held equal by a test.

Openness there had to be a decision rather than an accident, because the accident was
already in place and it was the worst kind. `flatten` forbids `deny_unknown_fields` on a
rule, and `Match` omits it so a project can compose its own criterion, so an unknown key
was silently dropped. Dropping the only key of a `match:` leaves the empty match — and the
empty match **is** the catch-all. So `match: { agent: t-writer }` became `match: {}`: the
rule answered every call, every rule after it was unreachable, the catch-all check saw a
catch-all and approved, and the case loaded green while proving nothing. `args_contains`
for `args_contain` did the same thing, and that one is a typo anybody makes.

The doc comment on `Match` had claimed for four lots that catching this was "the core's
job, at case load time, against the JSON schema". The core did not do it, and the schema
did not describe it. Third occurrence of a doc comment describing absent behaviour.

Two keys stay opaque and one does not, and the asymmetry is the point: `setup` is
interpreted by whichever adapter claims the case, so the core cannot know what belongs
there. `fake:` is interpreted by **our** engine, so anything it does not read is either a
typo or vocabulary that belongs in `setup` — and both deserve a sentence rather than
silence.

**Architecture Invariant:** every assertion carries the **path** it came from in the
document — `expect.files["…/plugins.yaml"].absent[0]`. The core needs no line
numbers, but pull-request annotation will later, and going from a path to a line is
easy whereas reconstructing a provenance you did not keep is not.

### `crates/gaveldrop` — `iso`

The isolated environment: a pristine directory per case, the home and configuration
directories redirected into it, `PATH` prefixed with the directory of symlinks to
the fake binary.

**Architecture Invariant:** a case never sees the real home directory. This is the
load-bearing invariant of the whole edifice — a defect here means the test suite
silently corrupts the actual configuration of whoever runs it. Every change to this
module is reviewed with that sentence in mind.

**Architecture Invariant:** an environment variable that could bypass the
redirection is **removed**, not merely overridden. A project that reads
`MYTOOL_CONFIG_DIR` before looking at the home directory short-circuits isolation if
that variable happens to sit in the environment of whoever runs the tests. The lesson
comes from the prototype, which explicitly removes its own.

**Architecture Invariant:** isolation asks **nothing** of the project under test. No
variable to read, no test mode, no injection point. It uses only what a process is
subjected to anyway: its environment and its search path.

A snapshot of the tree is taken after `setup` and before invocation; the difference
after invocation constitutes the "files" observation. **The observation takes
everything** — the directory is tiny, walking it costs nothing — **and the assertion
names paths.** So there is no trade-off between "full diff" and "watched list": they
are two different layers. As a bonus, the failure report lists files that were
written but that the case says nothing about — not as an error, but as help: that is
often where you discover what you should have been asserting.

### `crates/gaveldrop` — `adapters`

An adapter has a single responsibility: invoke the subject and return normalised
observations.

```rust
pub trait Adapter {
    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations>;
}

pub struct Observations {
    pub exit:   i32,
    pub stdout: String,
    pub stderr: String,
    pub files:  Vec<FileEffect>,
    pub calls:  Vec<Call>,
    pub ext:    BTreeMap<String, Value>,
}
```

**Architecture Invariant:** an adapter invokes and observes. It **never evaluates**.
No adapter knows what a case expects; it only fills in `Observations`. That is what
guarantees an expectation written once behaves identically whatever the technology.

**Architecture Invariant:** an observation records what the subject produced, and
**nothing we produced ourselves**. A note of our own goes in a field of its own.

Broken for one lot without a test noticing. A `capture:` whose path found nothing was
reported by appending a sentence to the step's `stderr` — a field that is otherwise
what the subject wrote, that already carries a real failure (a request that could not
be built), and that a case is entitled to assert on. So our commentary could satisfy a
`contains` or break an `absent` that had nothing to do with us, and it worked only
because an HTTP exchange happens to leave `stderr` empty.

The fix is also what made the diagnostic possible, which is the part worth keeping: a
missed capture is now `missed_captures` on the observations, and `verdict` turns it
into a failure at `steps[0] "creates the order".capture.order_id`. The adapter resolves
the path because it needs the value to substitute; deciding that a missing one is a
fault belongs to the evaluator, and that division is what gives the failure an
assertion path instead of a line nobody prints.

The symptom it removes: a reader saw a `404` two steps later, went looking at their
service, and the cause was one word in their own case. The `404` is still reported,
after the capture — a cause without its consequence leaves you wondering whether the
second request happened at all.

**Architecture Invariant:** an adapter fills `ext` only with what its technology
**alone** can produce. Anything observable of an arbitrary process already has a
named field. `ext` is not a junk drawer for whatever we lacked the nerve to place.

The `process` adapter — run a command, read what it produces — is the base case all
others are specialisations of. On its own it covers Rust, JavaScript/TypeScript,
Python, Java and Kotlin: in all five, the subject under test is a process, and the
fake binary is indifferent to the language of whoever calls it. The shell is the only
technology that needs an adapter of its own, because there you test a function rather
than an executable.

The trait has a single implementor at the start. It is there anyway: the shell and the
web both need it, and it is what the conformance kit puts under tension.

**Architecture Invariant:** an adapter is chosen by what the case declares, never by
configuration. A project mixing a binary and the shell scripts around it is ordinary
and must not have to split its suite. Each adapter answers `claims`, and the registry
is asked in order — never by trying `invoke` until one succeeds, which would run the
subject against an adapter that was not meant for it.

**Architecture Invariant:** whether a case can be invoked at all is decided by the
registry, not by the loader. `Case::load` used to refuse any `setup` without `run` or
`exec`, and the concern was right: a case that parses and then invokes nothing is a
green test asserting about a program that never started. But `run` and `exec` stopped
being the criterion the moment there was a second adapter, and `case` must not depend
on the adapters to know. So the refusal lives in `adapters::select`, which knows the
whole registry and names the keys it did find.

Moving it corrected an over-permissive check on the way: the loader accepted a case
with `setup.exec` alone, which prepares the directory and invokes nothing. Asking real
adapters instead of guessing at keys is what surfaced it.

**Architecture Invariant:** isolation carries the project root, and reading a project
file through it is not a breach. Some subjects **are** files of the project: a shell
function must be sourced from the repository to be the thing under test, so a relative
`source:` resolves against the project root rather than the isolated one. Writing would
be a breach and nothing permits it — the subject still runs with the isolated root as
its working directory, so anything it creates lands inside.

This was found by the repository's own cases, not by a unit test. The unit tests wrote
their fixtures *into* the isolation, so relative paths happened to work; a real case
naming a file in `tests/shell/` produced exit 127 with nothing on standard output. The
same blind spot had already hidden the hook resolution bug, for the same reason.

**Architecture Invariant:** `claims` has no default implementation. A default would
have to be either `true` — every third-party adapter claiming every case, with
registry order deciding silently — or `false`, an adapter that compiles and is never
chosen. Both fail quietly, and adding a required method to a published trait is the
kind of break that should be visible at compile time.

**Architecture Invariant:** a consumer-provided adapter that claims a case is the one
that invokes it, through the public runner. `runner::run_all_with` takes the adapters;
`run_all_selected` is the same call with `adapters::registry()`. The slice is searched
in order, so an adapter placed before the built-ins overrides them for its own cases.

This was absent, and the shape of its absence is worth keeping: the whole public
runner hardcoded `registry()`, while `run_one` — the only function taking adapters —
was private. So the conformance kit could **prove** an adapter that nothing was then
able to **use**. Everything a third party needed existed except the last inch, and
none of the internal tests could notice, because they all reach `run_one` directly or
test the built-ins through it.

The gap costs a paragraph to describe and had cost nothing to fix at any point in the
last sixty-nine changes. What it cost was a consumer, blocked, whose only two ways
forward were rewriting its cases to look like something they are not, or
reimplementing the loop this function contains.

### `crates/gaveldrop` — `verdict`

Evaluating expectations and invariants against `Observations`, and the weighted
score.

Core expectations: `exit_code`; `stdout` and `stderr` with `contains` and `absent`;
`files`, per path, with `contains` and `absent`; `calls`, by counts; `status`,
`headers` and `body` for a response. Plus reading the JSON lines emitted on standard
output, with subsequence order checking and per-type counts.

**Architecture Invariant:** `status`, `headers` and `body` belong to the core, and the
placement rule is what puts them there rather than an exception made for them. The
rule sends to an extension what **one technology alone** can produce; HTTP is answered
identically by a service written in Node, Rust, Python, Java or Kotlin. It is a
protocol several technologies share, not the property of any. That is what lets a case
be rewritten in another language without touching a single expectation — something
`shell:`/`source:`/`call:` could never offer, which is why those correctly live in
`Setup::extra`.

**Architecture Invariant:** one evaluator checks a step and the run as a whole. The
same `check` runs both, with the assertion path rooted differently. Two evaluators
would drift, and an expectation would then quietly mean one thing at the top level and
another inside a step.

**Architecture Invariant:** a mismatch between declared and performed steps is a
failure in **both** directions. Too few means the subject stopped halfway, and
comparing only what came back would report green. Too many means an exchange happened
that the case never declared — the same class of surprise as an unexpected call.

Header names are compared case-insensitively, in one place. A case asserting
`Content-Type` against a server sending `content-type` would otherwise be testing the
server's spelling rather than its behaviour.

**Architecture Invariant:** named invariants are not code written per project. There
are **four built-in shapes** — *paired*, *exactly one*, *no orphan*, *non-empty
field* — that the project configuration names and parameterises. Four, because those
are exactly the ones that existed in the prototype. A speculative invariant library
would be dead weight; a fifth shape gets added the day a real case demands it, not
before.

**Architecture Invariant:** a failure names the case, the expectation and the value
it got. A message that forces someone to open gaveldrop's code is a gaveldrop bug,
not a user inconvenience.

**Architecture Invariant:** the value a failure reports must not be able to read as a
*different* value. An excerpt shorter than what it excerpts says how much was left out;
control bytes are escaped rather than printed.

Both halves come from one incident, and it is the sharpest example of what this invariant
costs when it slips. `got` on a stream assertion showed the stream's **first line**. A
subject whose output began with a colour escape followed by a newline therefore produced a
`got` containing one invisible sequence — which the report rendered as nothing at all. An
empty `got` on a stream assertion means "the subject wrote nothing", so the reader
concluded their function was not running, reproduced it outside gaveldrop where it worked,
and wrote a probe case to interrogate the isolation. The function had been running the
whole time; the assertion was failing because they had deliberately made it false.

Note which way the failure went: not a missing message, but a message that answered a
question nobody asked, in the voice of one that had. A terminal *interprets* escapes, so
the bytes a diagnostic most needs to show are precisely the ones it hides.

`weight` per case surfaces the failures that matter; `allow_fail` tolerates known
cases without hiding them.

### `crates/gaveldrop` — `report`

The terminal output, the JSON report, the HTML report.

**Architecture Invariant:** what the engine decided is a **sink**, not a field of the
outcome and not a flag inside the terminal renderer. `Sink::preparing` is defaulted to
nothing, so every existing renderer is unchanged and the JSON, HTML and JUnit ones ignore
it — a trace of the engine is not a verdict and a dashboard has no column for it.

Not an observation either, and that is the same invariant as the one under `adapters`: an
observation records what the subject produced, a trace records what we did to it. Folding
it into `Observations` would have put our own words in the machine-readable report of every
run.

Printed **before** the case, because a case that hangs or takes the subject down with it
still leaves behind what it was about to do — which is when it is needed and when anything
printed afterwards never arrives.

Its contents are the four questions that actually cost time putting a real project on
gaveldrop, rather than a guess at what might help: which adapter claimed the case, where
the isolated root is, which tools are faked or hidden, and what the case's declared
variables resolved to. The whole environment is deliberately not dumped: twenty lines of
`XDG_*` per case would bury the four that matter.

**Architecture Invariant:** the JSON report is **a list of case outcomes plus a
summary computed from it**. Never a summary frozen at the top of the structure. That
is what makes two reports mergeable by plain concatenation, and therefore what will
make it possible to spread a suite across several machines without touching the
format.

**Architecture Invariant:** outcomes are emitted **as they happen, one per finished
case**, not only aggregated at the end. A report that only exists once the suite has
finished forecloses any live rendering — an editor ticking off its cases one by one, a
terminal showing a failure the moment it lands. Emitting one line per case costs a few
lines today; retrofitting it would mean turning the execution loop inside out.

### `crates/gaveldrop-cli`

The command-line facade, for projects that are not written in Rust. It reads the
project configuration, discovers the cases, executes, reports.

**Architecture Invariant:** the facade contains **no logic**. Everything it does is
available from the library. A behaviour that only exists by going through the binary
is a behaviour a Rust project cannot test.

### `crates/gaveldrop-conformance`

A battery of cases every adapter must pass to prove it honours the contract:
isolation did not leak outside the temporary directory, `Observations` is correctly
filled, an unexpected call trips the catch-all, the journal is complete.

The kit has two uses, and the second is the less obvious one: it stops the core from
deforming when a technology is added, and it gives a third party the means to
validate their own adapter without reading our code.

**Architecture Invariant:** the conformance kit is gaveldrop's own guarantee. A
particular consumer passing its tests **is not one**: those cases belong to the
consumer, they can change without notice, and copying them here would make them
diverge at the first change.

**Architecture Invariant:** every check must be refused by an adapter that breaks
what it guards, and hold for one that does not. Running the kit against a correct
adapter shows only that it can say *yes* — a kit whose checks all silently held
would look identical. `tests/refusal.rs` holds adapters that are deliberately
wrong for that reason, and they live outside the crate so that compiling them
proves the published API suffices for a third party. The first such adapter found
a check clearing a variable no environment defines: green whether the adapter
honoured `clear_env` or ignored it. A vacant check is worse than a missing one,
because it reads as coverage.

**Architecture Invariant:** an assertion about which checks refuse a broken
adapter may only be exact when that adapter's environment is fully determined. An
adapter leaking the ambient environment leaks what that environment holds, and
that differs between a developer's shell and a CI runner — pinning it encodes the
machine into the suite. The exactness proof belongs to an adapter that sets its
own environment. Learned from CI: a test pinning the leaky adapter to two
failures passed on macOS and was refused on Linux, where `XDG_CONFIG_HOME` is
set.

See `docs/conformance.md` for the checks and how a third party runs them.

### Where a case stops being data

**Architecture Invariant:** a case **names** a value and **substitutes** it. It never
**computes** one. No arithmetic, no conditionals, no iteration, no string manipulation
beyond substitution. A case needing any of those uses a hook — a real program, in a real
language, with a debugger. That is what the three hooks have always been for.

The need this concedes to is narrow and real: one exchange creates an order and gets back
`{"data":{"order":{"id":7}}}`, the next has to `GET /orders/7`. Without a way to carry that
id, the case cannot be written at all.

The temptation past it is a slope. A little arithmetic on a count. A conditional for the
optional field. A loop over the list in the response. Each step is individually reasonable
and the destination is a bad programming language with no debugger, embedded in YAML,
maintained by us.

**The mechanical test for a reviewer:** if a proposed addition would need a *second* value
to produce its result, it is computation and belongs in a hook. Naming needs one value.
Substituting needs one value. Adding, comparing, formatting and choosing all need two.

**Corollary:** a capture cannot shadow a variable isolation defines. `HOME` means the
isolated home in every case ever written, and a document able to redefine it would put the
load-bearing invariant in the hands of whoever writes the case. Isolation's names win, and
that is checked rather than trusted.

**Corollary:** a capture is visible only to *later* exchanges. A case reads top to bottom,
and a value used before it exists would make the order of the document a puzzle instead of a
sequence.

### The hooks — API Boundary

Three extension points, one protocol. The executable receives JSON on its standard
input and returns its result on its standard output. The isolated directory and the
case name reach it through the environment.

| Hook | Receives | Returns |
|---|---|---|
| `setup.exec` | the `setup` block | nothing — its exit code is the verdict |
| `fake.render` | the selected rule and the call | the bytes the fake must emit |
| `expect.exec` | the observations | `{ "ok": bool, "diffs": [...] }` |

**Architecture Invariant:** the unit of extension is **an executable**, not a Rust
crate. This is the decision that puts every targeted technology on equal footing: a
Kotlin or Python project can hook in exactly what a Rust project hooks in. Had the
extension point been a trait, only Rust could have extended gaveldrop.

**Architecture Invariant:** the contract is **the JSON protocol**, not the
convenience packages we will publish per ecosystem. A language with no package works
with three lines of `jq`, and a lagging package blocks nobody. No package ships until
a real project script has become ugly.

**Architecture Invariant:** three hooks. A fourth is a conscious decision justified
here, not a drift noticed six months later.

**An accepted cost:** `fake.render` is respawned on every intercepted call. A shell
script costs on the order of ten milliseconds, so a few seconds across a busy suite.
Noticeable but tolerable — and the cost only falls on projects with no alternative: a
Rust project builds its fake binary from the library and pays nothing.

### The path of a case

1. Load the case and validate it against the schema.
2. Create the pristine directory; redirect the home and configuration directories
   into it.
3. Lay down the symlinks to the fake binary, prefix `PATH`, write the scenario,
   create the counter and journal directory.
4. If the case has a `setup.exec`, run it in the isolated directory.
5. Snapshot the tree.
6. Let the adapter invoke the subject.
7. Collect: exit code, outputs, tree difference, journal.
8. Evaluate the expectations, then the invariants.
9. If the case has an `expect.exec`, run it and fold in its verdict.
10. Aggregate into the report.

## Cross-Cutting Concerns

### Code generation

A single artefact is generated: the case format's JSON schema, derived from the type
and committed. A test regenerates it and fails on divergence. This mechanism is
carried over from the prototype, where it proved it holds.

### Testing

Three levels, at three different boundaries, and they do not replace one another:

- **Unit**, closest to the code: rule selection, the counter, the expectation
  comparator, report merging.
- **The conformance kit**, at the adapter boundary: gaveldrop's own guarantee, and
  the only thing that stops the core from deforming when a technology is added.
- **Real consumers**, outside this repository: they are what carries the
  non-regression proof, each on their own ground. It is not repatriated here.

**Architecture Invariant:** a test writes only under a directory it created, or under a
path it has established belongs to this repository. Two tests write into the checkout on
purpose — the schema regeneration, which is how drift is caught — and that is legitimate
only because the checkout is ours.

The schema test resolved its target as `CARGO_MANIFEST_DIR/../../docs/`. In a checkout
that is the repository; in a package extracted from crates.io it is `~/.cargo/registry/`.
So the test read nothing, found a difference, **created a directory in a stranger's cargo
registry and wrote a file into it**, then failed. A test failing on someone else's machine
is a nuisance. A test writing outside its own tree there is a different category, and the
only reason it was never observed is that `cargo publish` verifies by compiling rather
than by testing.

It now asks whether `ARCHITECTURE.md` sits beside `docs/` before touching anything, and
the tests of the repository — the ones running our own cases — are excluded from the
published package, since a test that cannot pass where it is shipped is not a test.

Found by preparing publication, not by any of the three levels above. Which is the
recurring shape here: the defects come from doing the next real thing with the tool.

### Error handling

**Architecture Invariant:** a broken case never brings the suite down. A temporary
directory that refuses to be created, a program that will not start, a hook that
exits in error — all of it becomes a failed case with a diagnostic, not a panic that
takes the other ninety-nine cases with it.

The distinction matters: a **load** error is loud and stops everything (a malformed
case is a bug you need to see immediately), whereas an **execution** error is a case
failure like any other.

### Performance

One measurement drives the design, and it is the **startup time of the fake binary**.
It is respawned on every intercepted call; a serious suite has several hundred. Orders
of magnitude, empty startup:

| | startup | 500 calls |
|---|---|---|
| Rust or Go | ~2 ms | ~1 s |
| Node | ~35 ms | ~18 s |
| Python | ~40 ms | ~20 s |
| JVM | ~150 ms | ~1 min 15 |

That is the difference between a tool you run on every save and a tool you run while
fetching coffee. Since the project's promise is that coverage becomes cheap, a slow
tool is a tool people stop writing cases for. **The fake binary must therefore be
compiled** — which rules out Node, Python and the JVM for the core, independently of
anyone's taste.

Between Rust and Go, Rust wins for three cumulative reasons: a schema derived from
the type cannot lie, the first consumer is written in Rust and gets the typed path for
free, and the existing prototype is in Rust — so the first increment is a move rather
than a rewrite. Go would be a good choice for a project starting from nothing, and
would even be better for the web step.

### Observability

The HTML report is carried over from the prototype in the very first increment rather
than deferred. The reason is not technical: the code already exists, and a tool that
makes its first consumer regress by migrating starts off badly.

### Tool integration: CI and the editor are the same problem

Continuous integration and editor integration want the same thing under two guises.
Treating them as one problem avoids building the same plumbing twice.

**Three foundations, all in the core. No plugin in the core.**

**1. The schema derived from the type** covers writing a case all by itself —
completion, validation as you type, hover documentation — in **any editor that speaks
the YAML language server protocol**, without a line of code on our side. It is the
project's highest-return foundation, and it is already decided for another reason.

**Architecture Invariant:** doc comments on the case format types are not reading
comfort, they are **the tooltips seen in the editor**. They travel through the schema
all the way to the user. A badly documented field is a badly used field.

**2. Assertion provenance** — the path kept at load time, resolved into a line number
— serves exactly two consumers for one piece of work: the comment placed on the right
line of a code review, and the squiggle in the editor. Same data, same resolution.

**3. Outcomes as they happen**, plus machine-readable case discovery. That is
precisely what mainstream editors' test interfaces ask for: the list of cases, then a
stream of outcomes. A plugin then becomes a thin layer, and a thin layer is
maintainable.

**Architecture Invariant:** no editor plugin lives in this repository, and no
behaviour exists solely for a plugin. A plugin consumes case discovery and the outcome
stream — nothing else. That is the only way to avoid maintaining one plugin per editor
per version.

Practical corollary: watch mode — rerunning affected cases on every save — is what
makes choosing a compiled fake binary pay off day to day. Without it, the millisecond
saved per call shows up nowhere.

### The genericity verdict, measured on the shell

The shell was picked as the arbiter: a technology whose subject is a **function**, not a
process. The bet was that the core would barely grow. Here is what happened, since a
lot whose stated purpose is to test a bet has to report the result including a loss.

**What did not move, and this is the part that matters.** `Case`, `Expect`,
`Observations`, `verdict`, `report`, the YAML format and the generated schema are
untouched. Not one shell word entered them: `shell`, `source` and `call` arrive through
`Setup::extra`, which is opaque past `run` and `exec` by design, and only the adapter
reads them. An expectation written once means the same thing whether the subject is a
binary or a function — and that is not an assertion, it is what the conformance kit
demonstrates by passing the same six checks against both adapters.

**What grew, twice.**

1. `Adapter::claims`, plus `registry` and `select`, replacing a hard-coded
   `Process.invoke` in the runner. Foreseen: a second adapter makes hard-coding a bug
   rather than a simplification, and the web needs the same thing next.
2. `Isolation` now carries the project root. **Not** foreseen. It is not about selection
   but about what isolation is allowed to know, and it exists because some subjects *are*
   files of the project rather than executables on a path.

**What was wrong and got corrected on the way.** `Case::load` refused any `setup` without
`run` or `exec`, which made a shell case unloadable before any adapter could claim it —
so `extra` was opaque to serde and not to the core. Moving that refusal into `select`
also revealed the check had been too permissive in the other direction: it accepted
`setup.exec` alone, which prepares a directory and invokes nothing.

**The verdict.** The core is generic in its **vocabulary** and was not in its **wiring**.
`extra` and the `Adapter` trait absorbed a foreign technology without a single domain word
entering the format, but two plumbing decisions had been written as though there would only
ever be one adapter. Both were about *how a case reaches its subject*, neither about *what
an expectation means* — and the second question is the one the whole project rests on.

Worth recording for the web adapter: every growth this lot paid for was found by running
gaveldrop against its own cases, never by a unit test. The unit tests wrote their fixtures
*into* the isolation, so relative paths worked there by accident. That blind spot had
already hidden the hook-resolution bug. A technology is not absorbed until a real case
runs through it.

### The genericity verdict, measured on the web

Lot 5 tested whether the core was generic in its **vocabulary**. This one tested something
else: whether it is generic in the **shape of a case**. A living subject interrogated
several times is the first thing that does not fit "one invocation, one verdict".

**What did not move.** The rule engine, the counter, the journal, the catch-all, the
isolation, the reports, the conformance kit's six checks. The fake answers from the same
`fake.rules` at either door. `Observations` and `Expect` grew fields but changed no shape.

**What grew, and it was the format this time.** `steps:` is part of the case document, not
one technology's vocabulary — invoking a subject twice is observable of any process.
`status`, `headers` and `body` joined the core expectations, and the placement rule put
them there rather than an exception: HTTP is answered identically by a service in Node,
Rust, Python, Java or Kotlin, so it is not what "one technology alone can produce".

`Isolation` gained two ports and the project root as a variable. The `Adapter` trait did
**not** change — the per-step observations are nested in `Observations` precisely so it
would not, after growing twice in lot 5.

**What the lot got wrong twice, the same way.** A doc comment described behaviour the code
did not have: a request body documented as reaching the matcher while an empty string was
passed, and a TCP readiness fallback documented while the function returned `false`
unconditionally. The second cost a two-minute test timeout. Both are now in `AGENTS.md`.

**The verdict.** The shape of a case *did* have to grow, and that is the honest answer to
the question this lot asked. But it grew in the format rather than in any adapter, and
what it added is available to every technology: a Rust binary can be invoked twice with
`steps:` today, with no web anywhere in sight. The core absorbed a living subject by
becoming more general, not by learning about HTTP.

One thing was deliberately **not** built: carrying a value between steps. It is where a
case format starts becoming a programming language, and the line is better drawn once real
cases show what they need. Recorded in `ROADMAP.md` for lot 6b rather than guessed at here.

### The genericity verdict, measured on GraphQL and state

Two lots asked whether the core bends to a technology. This one asked a different
question, and got a different kind of answer: **two of its five items needed no code at
all**, and one needed a decision rather than a feature.

| Item | Outcome |
|---|---|
| GraphQL expectations | `json:` — a JSON path, which serves REST identically. No `graphql:` key |
| Carrying a value between steps | `capture:`, plus the invariant that is the real deliverable |
| `idempotent:` | **Will not exist.** Two identical steps, the second declaring `no_new_files:` |
| An API's database | Answered by `setup.exec`. Documented, not built |
| `render:` at the HTTP door | One function, sharing the hook protocol with the binary door |

A lot that ships nothing for an item and does not say why looks like an oversight, so each
of those is written down where someone will find it: in `docs/web.md` for a reader writing
cases, and here for a reader changing the code.

**What grew.** `json:`, `capture:`, `no_new_files:`, `Case::expect` defaulted, and the shell
adapter learning `steps:`. Every one of those is available to **every** technology: a Rust
binary can be invoked twice with `steps:` and asserted on with `json:` today, with no web
anywhere in sight. That is the same result lot 6 had — the core absorbs by becoming more
general, not by learning a domain.

**What this lot's real deliverable was.** Not a feature. The line in "Where a case stops
being data", and the mechanical test that goes with it: if a proposed addition would need a
*second* value to produce its result, it is computation and belongs in a hook. Every item
above was decided against that line, and `idempotent:` was refused by it.

### What continuous integration asked of decisions made earlier

This lot built almost nothing new. Every piece reads what the core already produced, which
makes it a bill arriving for four decisions taken in lots 2 and 4.

| Decision | Where it paid |
|---|---|
| Assertion paths kept, without line numbers | `report::lines` turns one into a line, so an annotation lands on the assertion that broke |
| A report stores outcomes; the summary is computed | shards concatenate, and `cat` is the whole merge step |
| The terminal renderer is one `Sink` among several | JUnit and annotations joined without touching the runner |
| `Sink` has both `case_finished` and `finish` | JUnit needs the second: its header carries the totals, so it cannot stream |

Three of those cost nothing to collect. The first has a price, and it is worth naming now
that it can be measured.

**The price of not keeping spans.** `serde_yaml_ng` reports positions only in its errors, so
a parsed document carries none, and recovering provenance means re-reading the file and
walking it by indentation. That is about two hundred lines with genuine edge cases —
`data.order.id` is one YAML key containing dots rather than three levels, and a bracketed
file path contains dots that are not separators. Both produce annotations on *nearby* lines
when wrong, which is the kind of wrong nobody notices.

A parser carrying spans would have given exact lines for free. It would also have meant a
different dependency from lot 2 onwards, and coupling the loader to it. The choice still
looks right — but it was not free, and a future lot wanting exact editor squiggles should
weigh the resolver's edge cases rather than assume the path-to-line step stays cheap.

**What did not need a decision at all.** Gating, sharding and selection are all functions of
the report and the discovered paths. That they slotted in without touching a single adapter
is the placement rule holding at a distance: none of them knows what a technology is.

### Nomenclature

**Everything in this repository is in English** — identifiers, format keywords, doc
comments, error messages, test names, commit messages, documents. The name is
reserved on the public registries, and a second language would be a wall for anyone
arriving from outside. One language, no boundary to remember.

A word on vocabulary: "e2e" describes poorly what we do for a technology like the
shell, where a function is tested with its dependencies faked — that is closer to an
integration test. Command and documentation naming should account for it.

### The genericity verdict, measured on a real consumer

The three verdicts above were measured by us, adding a technology to our own repository.
This one was measured by somebody else putting a real project on the format, and it is the
only one that could not be arranged to come out well.

zanvil — a Zsh configuration suite, first real consumer of the shell adapter — reached 26
cases at `104/104` on Linux and macOS, replacing four CI steps of which **three could not
fail**. It then went through what was left in bash, twenty assertions, and reported that
**none of them waits on a key of gaveldrop**: fourteen need only `setup.stdin` and
`fake.bins`, four are a convention internal to their own tool, and the rest are a fixture
contract better expressed as an equality than as a line count.

What the format cost to get there, honestly: **three keys and one message**. `equals`,
because `contains` on a measurement is not a weak assertion but a false one — `contains: 2`
passes on a result of `12`. `setup.stdin`, because a filter was the one shape a case could
not invoke: argv, environment and search path were all controllable, standard input was not.
`ignore_ansi`, because a tool that colours every field breaks a substring match on the
escapes between the words. And a gate message that distinguishes an unreachable threshold
from a failing suite.

Two of the three are things any technology has — an input, an output that may be styled —
which is why they belong in the core rather than in the shell adapter. Neither the shell
adapter nor `Observations` grew a line for them.

The number that matters is the one nobody set out to produce: on eight findings from their
first report, **six became code**, and two were refusals with reasons they then agreed with.
The defects were real and none had been found by a unit test.

## What does not exist yet

Each increment ships something usable on its own.

1. **The fake engine** — `gaveldrop-fake`. **Done.** Rule matching, the per-key call
   counter, the append-only journal, the four response modes, scenario loading, and the
   binary. 54 tests, 15 of them end to end against the real binary, green on Linux and
   macOS.
2. **The core** — `case`, `iso`, `adapters`, `verdict`, `report`, with `process` as the
   only adapter, plus the CLI facade and the conformance kit. Not started. Too large for
   one plan; expect two or three.
3. **The shell** — `adapters::shell`. **Done.** Sourcing configuration files in order,
   invoking a function, faking its dependencies, observing files dropped under the
   isolated home. Six cases of our own run through it, in `zsh`, on Linux and macOS. The
   verdict on genericity is below, and it is not a clean win.
4. **The web.** A living subject — start it, wait until it is ready, stop it cleanly,
   reserve a port — multi-step cases, and a second door for fakes: a server that
   listens instead of a binary on `PATH`. The rule engine is the same; only the door
   changes. Placed third because it is the step that adds the most machinery, and we
   will write it better with two technologies already behind us.
5. **Continuous integration.** JUnit XML, code-review annotations pointing at the
   case's line, gating thresholds, selection and sharding across machines. A GitHub
   Action is essentially thirty lines on top of the binary, because an annotation is
   one line of text on standard output. This is where assertion provenance becomes a
   line number.
6. **Distribution and plugins.** Publishing the crate and the binary, publishing the
   schema, integration documentation, watch mode, editor plugins, and the per-ecosystem
   convenience packages — each if a real need calls for it. A reminder: writing a case
   is already covered in every editor from increment 1, purely because the schema is
   published. A plugin only adds running and visual feedback.

Two points stay open and are deliberately left that way:

- **Carrying a value between steps** ("keep the identifier the first request
  returned"). Indispensable for testing an API, and exactly the place where the YAML
  will start wanting conditionals and computation. It will be the first place we have
  to say no.
- **An API's database.** You do not isolate it by moving a home directory. The setup
  hook does the work — but that is delegation, not a solution.
