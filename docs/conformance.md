# The conformance kit

An adapter's job is to invoke the subject and report what happened. It never evaluates: no adapter
knows what a case expects. That is what lets an expectation written once behave identically whatever
the technology — and it is also what makes a broken adapter dangerous. An adapter that quietly loses
the exit code does not fail. It makes every case pass.

The kit is the battery that catches that. Run it against your adapter before you trust a single
green case.

## Running it

```rust
#[test]
fn my_adapter_is_conformant() {
    let report = gaveldrop_conformance::run(&MyAdapter, &path_to_gaveldrop_fake);

    assert!(report.is_conformant(), "{}", report.render());
}
```

The fake binary is passed in rather than located, so you can point at a fake you built yourself from
`gaveldrop-fake` as a library — which is how a project supplies its own response rendering.

A refusal prints the checks that failed first, each with what it protects and what was seen:

```
FAIL the_home_directory_is_the_isolated_one
     why  this is the load-bearing invariant: an adapter that lets the subject see the real
          home makes the suite corrupt the configuration of whoever runs it
     got  the subject saw HOME="/Users/you"
ok   exit_code_is_reported
ok   both_streams_are_reported
```

The reason is printed rather than the name alone. Fixing an adapter means knowing what the check
guards, and sending you to our source to find out would defeat the point of shipping a kit.

## What it checks

| Check | The defect it catches |
|---|---|
| `exit_code_is_reported` | an adapter always reporting zero makes every case pass |
| `both_streams_are_reported` | merging the streams makes an `absent` on one silently read the other |
| `the_home_directory_is_the_isolated_one` | the load-bearing invariant: the suite corrupting the configuration of whoever runs it |
| `a_cleared_variable_does_not_reach_the_subject` | isolation built correctly, `clear_env` skipped |
| `files_written_are_reported` | the `files` family of expectations made vacant |
| `an_unexpected_call_reaches_the_catch_all` | the fake not first on `PATH`, so every case calls the real tool |

## Why the kit tests itself with adapters that are wrong

That the kit passes a correct adapter proves it can say **yes**. It is not the property that
matters: a kit whose every check silently held would look exactly the same. So the kit is run
against two adapters that are deliberately broken, in `tests/refusal.rs`.

`Leaky` runs the subject in the isolated directory with the ambient environment. It looks correct —
the exit code, both streams and the files are all faithful. Only the environment is the developer's
own, so the subject reads the real home and the real search path. It must fail
`the_home_directory_is_the_isolated_one` and `an_unexpected_call_reaches_the_catch_all`, and the
three checks that have nothing to do with the environment must still hold.

`Forgetful` builds the isolated environment correctly and skips `clear_env`. A subtler mistake, and
a likelier one. It must fail `a_cleared_variable_does_not_reach_the_subject` and **nothing else**.

A kit that refused every adapter, including the correct one, would measure nothing; and a report
naming six failures when one thing broke is useless for repairing an adapter. So the exactness
matters — but it is `Forgetful` that establishes it, and there is a reason for the division of
labour.

`Leaky` cannot. An adapter that leaks the ambient environment leaks whatever that environment
happens to hold, and `XDG_CONFIG_HOME` is set on GitHub's Linux runners and unset on macOS. The
environment check therefore fails against `Leaky` on one platform and holds on the other —
correctly in both cases. Asserting it either way would encode the machine that ran the suite into
the suite. `Forgetful` sets the isolated environment itself, so it depends on nothing outside, and
that is what makes an exact assertion honest there.

This one is not hypothetical: the first version of the test pinned `Leaky` to exactly two failures.
It passed on macOS and CI refused it on Linux.

That test found a real defect the day it was written. The environment check used to clear
`CONFORMANCE_PROBE`, a name no environment defines — so it was removed whether the adapter applied
`clear_env` or ignored it entirely. Six checks green on `Forgetful`. The check now clears
`XDG_CONFIG_HOME`, which the isolation itself sets, so skipping the list is visible.

A vacant check is worse than a missing one: it reads as coverage.

## Writing an adapter outside this repository

The broken adapters live outside `gaveldrop-conformance` on purpose. Compiling them proves the
published API is enough — that a third party never has to reach for anything private.

You need `Adapter`, `AdapterError`, `Case`, `Isolation`, `Observations`, and `Journal` to read the
call journal back. All are exported from the crate root.

**End every struct literal with `..Default::default()`.** `Setup`, `Observations` and
`TextExpectation` all derive `Default` and all gain fields — `env`, `hide`, `stdin`,
`missed_captures`, `equals`, `ignore_ansi` all arrived after the first consumer wrote its adapter.
None of them is `#[non_exhaustive]`, so an exhaustive literal stops compiling on the next addition,
and a consumer depending by path feels that the moment they pull. Our own adapters do this, which is
why none of them changed when those fields landed.

### Gate the dependency if your support code is its own workspace member

This one was found by a consumer, not by us, and it will catch anyone in the same shape. A crate of
yours that holds your fake engine or your adapter — a workspace member alongside your binary —
pulling in a gaveldrop crate as a plain dependency:

```toml
# crates/my-fake/Cargo.toml
[dependencies]
gaveldrop-fake = { path = "../../gaveldrop/crates/gaveldrop-fake" }
```

reaches a **release build of your whole workspace**:

```console
$ cargo build --release            # no -p, which is what CI usually runs
   Compiling my-tool v1.0.0
   Compiling gaveldrop-fake v0.1.2   # <- came along
   Compiling my-fake v0.0.0
```

No feature on your *binary* prevents it. `cargo build` with no `-p` builds every workspace member as
its own top-level target, independently of how carefully the binary gates its own dependency on that
member. The member's own edge is what has to be gated:

```toml
[features]
engine = ["dep:gaveldrop-fake"]

[dependencies]
gaveldrop-fake = { path = "…", optional = true }
```

with `#[cfg(feature = "engine")]` on whatever touches it, and `features = ["engine"]` on the edges
that genuinely need it. `cargo tree -e normal,build -i gaveldrop-fake` should then print nothing for
your release configuration.

Nothing about gaveldrop's crates causes this — they are ordinary crates and behave the same either
way. It is a Cargo shape that only shows up once your test support lives in its own member, which is
exactly where a custom adapter ends up.

`Isolation` gives you everything and interprets nothing:

- `root()` — the directory to run in
- `env()` — the variables to set
- `cleared()` — the variables to remove, which is not the same as overriding them: a subject
  checking whether a variable *exists* must find that it does not
- `journal_path()` — where the fake wrote what it was asked for
- `changes()` — what the subject wrote under the root

Apply all of them. Each check in the table above exists because skipping one of them is invisible
until a case that should have failed goes green.

## When your subject is not a script

Every check hands the factory a shell script and expects the subject to run it: `exit 7`,
`echo err >&2`, `printf %s "$HOME"`, `printf hello > written.txt`. The shell adapter satisfies them
because its subject *is* the script.

An adapter whose subject is a fixed program cannot. `mytool run <fleet>` cannot be made to exit 7 or
to write `written.txt`, so there is no case shape that makes the checks pass through the real
invocation path. That is a limit of the kit, not of your adapter.

What the checks actually assert is narrower than "your subject ran": the environment was applied,
the exit code survived, the streams stayed apart, the journal and the file changes were read back.
That is **plumbing**, and it is shared between running a fleet and running a script. So factor it
out and let the factory drive it:

```rust
/// Applies the isolation and collects what came back. The only place that does.
fn run_in_iso(command: &mut Command, iso: &Isolation) -> Result<Observations, AdapterError> { … }

impl Adapter for MyAdapter {
    fn invoke(&self, case: &Case, iso: &Isolation) -> Result<Observations, AdapterError> {
        // a conformance probe carries its script; a real case carries the fleet
        let mut command = match case.setup.extra.get("conformance_script") {
            Some(script) => shell_command(script),
            None => self.fleet_command(case),
        };
        run_in_iso(&mut command, iso)
    }
}
```

**The branch has to end in the same `run_in_iso`, and that is the whole condition.** An adapter with
a conformance-only path that applies the isolation *its own way* gets six green checks about code no
case will ever execute — a vacant kit, which reads as coverage and is worse than no kit at all. That
failure mode is the reason `Leaky` and `Forgetful` exist above; do not reintroduce it in your own
adapter.

What this pattern does not prove is that your real invocation is correct — that your command line is
built right, that the fleet starts. That was never conformance's job; it is what your own cases are
for.

## Bringing your own fake

The other half of the custom-adapter story, and the one nothing here told: if your subject calls a tool
whose responses need shaping — a wire format, a streaming protocol — you build **your own fake binary**
on `gaveldrop-fake` as a library.

```rust
// src/bin/my-fake.rs
fn main() {
    let scenario = my_own::Scenario::from_env().unwrap();     // your rule type, your criteria
    let invocation = Invocation::from_env(scenario.needs_stdin()).unwrap();
    let call = Counter::at(&state_dir).next(&key).unwrap();

    let rule = scenario.select(&invocation, call);            // your matcher
    Journal::new(journal_path).record(&Call { … }).unwrap();  // ours, unchanged

    print!("{}", my_own::render(rule));                       // your bytes
}
```

`Counter`, `Journal` and `Invocation` are ours and need no reimplementing — the per-key counter and the
append-only journal are not specific to anyone. What is yours is the criterion (armadai matches on an
agent name, which is a word gaveldrop must never learn) and the rendering.

Then pass it where the runner asks for a fake binary:

```rust
let fake = PathBuf::from(env!("CARGO_BIN_EXE_my-fake"));
runner::run_all_with(&config, root, &fake, &mut sink, None, &[], &chain)
```

`locate::fake` finds **ours**, so it is of no use to you — you know where yours is, and
`CARGO_BIN_EXE_*` hands it over with no path to hardcode.

**Your fake is also the kit's fake.** `run_with` takes the fake binary it should symlink into place, so
the conformance checks call yours. That is why one of them —
`an_unexpected_call_reaches_the_catch_all` — depends on your fake behaving when no rule matches: it must
journal the call and exit non-zero, the way ours exits 127. A fake that answers silently there makes the
check pass while proving nothing.

**`require_catch_all` takes booleans, not rules**, precisely so you can reuse it with a criterion of your
own. Call it at load time and you inherit the refusal that keeps a scenario meaningful.

## Running your suite through your adapter

Passing the kit proves the adapter. Running the suite is a second function, because the `gaveldrop`
binary cannot reach an adapter compiled into your crate — so a project with its own adapter drives
its suite from a Rust test:

```rust
use gaveldrop::adapters::{self, Adapter};
use gaveldrop::report::terminal::Terminal;

#[test]
fn the_suite_passes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = gaveldrop::Config::load(&root.join("gaveldrop.yaml")).unwrap();

    let mut chain: Vec<Box<dyn Adapter>> = vec![Box::new(MyAdapter)];
    chain.extend(adapters::registry());

    let mut sink = Terminal::plain(std::io::stdout());
    let report = gaveldrop::runner::run_all_with(
        &config, root, &fake_binary, &mut sink, None, &[], &chain,
    )
    .unwrap();

    assert!(report.is_success(), "{} case(s) failed", report.summary().failed);
}
```

**A value your adapter needs *during* an exchange, it has to read itself.** The runner extracts structured
events after `invoke` returns — that is why an adapter needs no `EventsConfig` and gets none — so anything
an exchange must know before the next one starts is the adapter's to read out of the response it just
received. That is exactly what `capture:` is for the web adapter, and the shape a custom adapter follows
for its own equivalent. Reported by the consumer who removed a per-step event extraction after the runner
started doing it, and kept a smaller read for precisely this reason.

This example is the doc comment on `run_all_with`, where cargo compiles it — a code sample in a
markdown file is checked by nobody, and the first version of this one used two methods that do not
exist.

The slice is searched in order, so `MyAdapter` claims what it recognises and the built-ins keep
everything else — a project mixing its own vocabulary with plain `run:` cases needs no second suite.
Drop the `extend` if you want only yours. Sharding is the `None` and `--only` is the empty slice, and
every sink is available, so a CI report is the same code as it would be from the command line.

This function was missing until a real consumer needed it, and the shape of the omission is the
useful part. The broken adapters above prove a third party can *write* an adapter with the published
API — they are passed to the kit, which has always taken one. Nothing proved a third party could
*run* one, because every internal test reaches the private `run_one`. The kit could certify an
adapter that no public entry point would then accept.
