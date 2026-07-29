# gaveldrop-conformance

The battery every gaveldrop adapter must pass.

An adapter invokes the subject and reports what happened; it never evaluates. That is what
makes an expectation written once behave identically whatever the technology — and it is also
what makes a broken adapter dangerous, since one that quietly loses an exit code does not fail,
it makes every case pass.

Run this against your adapter before you trust a single green case:

```rust
let report = gaveldrop_conformance::run(&MyAdapter, &path_to_gaveldrop_fake);
assert!(report.is_conformant(), "{}", report.render());
```

A refusal prints what each check protects and what was seen, so fixing an adapter never means
reading our source.

See [gaveldrop](https://github.com/Dr0drigues/gaveldrop) and `docs/conformance.md`.
