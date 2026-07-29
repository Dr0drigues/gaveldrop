# gaveldrop-fake

The rule engine behind gaveldrop's faked dependencies: matching a call, counting it per key,
journaling it, and answering.

Both a **library** and a **binary**. The binary is symlinked under the name of each tool to
fake and placed first on `PATH`, so the subject under test finds it without knowing anything
has changed. The library is there because a project needing its own response rendering builds
its own binary on top of it.

It depends on no other crate in this repository. A consumer that only wants the engine does not
pull in the evaluation, the reports or the case format.

See [gaveldrop](https://github.com/Dr0drigues/gaveldrop) for what this is part of.
