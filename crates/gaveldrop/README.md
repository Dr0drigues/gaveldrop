# gaveldrop

A test engine where **one case is one YAML file**. A case says how to invoke a program, how its
dependencies must respond, and what the result must contain.

This crate is the core: loading cases, preparing an isolated environment, asking an adapter to
invoke, and evaluating expectations against normalised observations. Three adapters ship with
it — a process, a shell function, and a living HTTP service.

The core knows no language, no framework and no tool. It knows processes, files and lines of
text.

See [gaveldrop](https://github.com/Dr0drigues/gaveldrop) for the documents that explain why.
