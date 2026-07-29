# gaveldrop-cli

The `gaveldrop` command.

```sh
gaveldrop                                   # run every case the configuration finds
gaveldrop --only an-order                   # just the ones whose path matches
gaveldrop --shard 1/3 --report-json a.jsonl # one slice, for a CI matrix
gaveldrop --annotate --report-junit j.xml   # annotate a pull request, feed a dashboard
```

It contains **no logic of its own**: everything it does is available from the `gaveldrop`
library, which is what lets a Rust project test the same behaviour without going through a
process.

See [gaveldrop](https://github.com/Dr0drigues/gaveldrop) for the whole picture.
