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
own, so the subject reads the real home and the real search path. It must fail exactly
`the_home_directory_is_the_isolated_one` and `an_unexpected_call_reaches_the_catch_all`.

`Forgetful` builds the isolated environment correctly and skips `clear_env`. A subtler mistake, and
a likelier one. It must fail exactly `a_cleared_variable_does_not_reach_the_subject`.

**Exactly** is the word doing the work. A kit that refused every adapter, including the correct one,
would measure nothing; and a report naming six failures when one thing broke is useless for
repairing an adapter.

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

`Isolation` gives you everything and interprets nothing:

- `root()` — the directory to run in
- `env()` — the variables to set
- `cleared()` — the variables to remove, which is not the same as overriding them: a subject
  checking whether a variable *exists* must find that it does not
- `journal_path()` — where the fake wrote what it was asked for
- `changes()` — what the subject wrote under the root

Apply all of them. Each check in the table above exists because skipping one of them is invisible
until a case that should have failed goes green.
