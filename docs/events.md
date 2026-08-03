# Structured events

A subject that prints JSON lines on standard output tells you what it *did*, not just what came out
of it. That is a different kind of assertion from `stdout.contains`: the order things happened in, how
many times each happened, and the rules that must hold across all of them.

Events live in the **core** rather than in an adapter, by the placement rule this project is built on:
JSON objects on standard output are observable of *any* process. A technology that emits them gets
event assertions for free, and one that does not loses nothing — an omitted expectation is never
checked.

Everything on this page is exercised by
[`tests/cases/a-run-that-emits-events-is-provable.yaml`](../tests/cases/a-run-that-emits-events-is-provable.yaml),
which is where these examples were copied from.

## Declaring the vocabulary

The core learns nothing about your event types. It needs one thing: which field names them.

```yaml
# gaveldrop.yaml
events:
  type_field: t
```

`t` in this repository, `event` or `kind` or `type` in yours.

Omit the whole block when your subject emits no events. With no block, no line is read as an event, so
a case naming one in `expect.events` fails — which is the right answer to a case that assumed a
vocabulary the project never declared. Watch the one exception: `event_counts` with a `0` still passes,
because zero events of a type is exactly what it asserts.

## What counts as an event

Every line of standard output that parses as a JSON **object** and carries a **string-valued** type
field. Everything else is skipped without complaint:

```
{"t":"run_start","v":1}
starting, in prose nobody parses      ← not an event, not an error
{"t":"step_start","step":"fetch"}
{"level":"info","msg":"connected"}    ← structured, but no `t`: not an event
```

That tolerance is deliberate. A program mixing human output with structured lines on one channel is
the normal case, and structured logging that happens to share the stream must not be mistaken for
events. It also means a case that asserts on events keeps working the day someone adds a log line.

## `expect.events` — what happened, in order

```yaml
expect:
  events:
    - { t: run_start }
    - { t: step_end, step: fetch, items: 3 }
    - { t: result, cost: 0.0 }
```

Two things are going on, and both are subsets.

**The list is a subsequence, not an exact list.** Other events may occur before, between and after
the ones you name. Demanding an exact list would make every case break the day the subject gains one
new event, which is how event assertions get deleted rather than maintained.

**Each entry is a subset of one event's fields.** `{ t: run_start }` matches a `run_start` carrying
twenty other fields. You name what you care about; silence is not a claim.

Order is checked and repetition works: two `{ t: vote }` entries require two distinct `vote` events,
in that order.

### Numbers, and their two spellings

`cost: 0.0` above matches an event that emitted `"cost":0`. It has to: JSON has one number type and
its spellings are not interchangeable in a Rust deserialiser — YAML `0.0` is a float, JSON `0` is an
integer. Which one reaches your case depends on what language the subject is written in
(`JSON.stringify(0.0)` emits `0` where `serde_json` emits `0.0`), and that is not something the person
writing a case has any reason to be thinking about.

Two **integers** are still compared exactly, so an identifier past 2⁵³ cannot compare equal to a
different one. And a number never matches a string: the day a subject starts quoting its costs is a
real failure.

### What a failure tells you

```
expect.events[1]
  expected  { items: 4, step: "fetch", t: "step_end" }
  got       the closest was event 3 of 4, where items is 3, not 4
```

The event that shares the most of your fields is the one named, which needs no configuration — the
type field is one field among the others, so an event of the right type is already the closest. A
field the subject never emitted is reported as *absent* rather than as a wrong value, because that
usually means the case named it wrong. When nothing after the previous match shares a single field,
you get the plain sentence instead: there is no near miss worth pointing at.

An event that exists but sits **behind** the walk says so rather than saying it is missing:

```
expect.events[1]
  expected  { t: "a" }
  got       an event matching this is at position 1, before the previous expectation matched. Events
            are checked in order, so one of the two lists is out of order — the case's or the
            subject's
```

Which list is wrong is left open on purpose: a case that lists two events the wrong way round and a
subject that emits them the wrong way round are both real, and only you know which you meant.

Only the first broken position is reported. Once a subsequence breaks, later positions mean nothing.

## `expect.event_counts` — how many times

```yaml
expect:
  event_counts:
    step_start: 1
    step_end: 1
    retry: 0
```

Exact counts, per event type. **A declared `0` proves an event never happened** — that the retry did
not fire, that the budget warning was not emitted. No other assertion here can say that: a
subsequence says what did occur, and `stdout.absent` matches text rather than structure.

A type you do not mention is not counted.

## `expect.invariants` — the rules that hold across everything

Some properties are not about one event. *Every* agent that started also ended. There is exactly
*one* result. No step is referenced before it was declared. Written as expectations these would be
one entry per occurrence, and they would be wrong the moment the subject did something the case did
not enumerate.

So a project names them once, in its configuration, and a case uses the name:

```yaml
# gaveldrop.yaml
invariants:
  step_start_end_symmetric: { shape: paired, start: step_start, end: step_end, key: step }
  single_result:            { shape: exactly_one, type: result }
  step_name_non_empty:      { shape: field_non_empty, type: step_start, field: step }
  no_step_before_its_start: { shape: no_orphan, key: step, root: step_start }
```

```yaml
# the case
expect:
  invariants:
    - step_start_end_symmetric
    - single_result
```

The name is what a failure is reported under — `expect.invariants.step_start_end_symmetric` — which is
the point of naming them. "The paired shape failed" would send the reader to the configuration to work
out which one.

### The four shapes

| `shape` | Parameters | Holds when |
|---|---|---|
| `paired` | `start`, `end`, `key` | every `start` has an `end` carrying the same `key` value, and no `end` is unmatched |
| `exactly_one` | `type` | there is exactly one event of that type — not zero, not two |
| `field_non_empty` | `type`, `field` | every event of that type carries `field`, present and not empty |
| `no_orphan` | `key`, `root` | every event carrying `key` was **preceded** by a `root` event with the same value |

Four because those are the four a real project needed. A fifth gets added the day a real case demands
one; a speculative invariant library would be dead weight.

`no_orphan` walks the events in order rather than comparing sets, because a key used *before* it was
declared is exactly the bug it exists to catch.

`field_non_empty` takes **one** field on purpose. A project wanting "every `agent_start` carries both a
provider and a model" declares two named invariants rather than one taking a list. That costs a line of
configuration and buys the diagnostic: the failure says which of the two was missing, where a
`prov_and_model_non_empty` would only say that one of them was.

A failure names what was wrong and, where a count is the answer, how many:

```
expect.invariants.step_start_end_symmetric
  expected  holds
  got       step_start without step_end: ["", "publish"]; step_end without step_start: ["nope"]
expect.invariants.single_result
  expected  holds
  got       0 events of type result, expected exactly one
expect.invariants.step_name_non_empty
  expected  holds
  got       1 step_start events with step missing or empty
expect.invariants.no_step_before_its_start
  expected  holds
  got       used before any step_start: {"nope"}
```

**Every invariant is reported, not only the first.** Unlike a subsequence — where later positions mean
nothing once the order breaks — four broken invariants are four independent facts, and fixing them one
run at a time would be three runs wasted.

A name the project never declared is its own failure rather than a silent pass:

```
expect.invariants.typo_in_the_name
  expected  an invariant the project declared
  got       typo_in_the_name appears in no `invariants:` block. Declare it in gaveldrop.yaml, or fix the spelling
```

## What a case cannot do

**Compute a value.** A case cannot assert that `tokens_in + tokens_out == total`. Arithmetic across
events is what an invariant shape is for, and adding an expression language to the case format would
trade the property the whole project rests on — that a case is readable and writable by hand — for
something a `paired` or a new shape does better. `ARCHITECTURE.md` records the reasoning.

**Assert an exact event list.** By design, as above.

**Gate on how long anything took.** Durations are reported everywhere and asserted nowhere; see
`docs/ci.md`.
