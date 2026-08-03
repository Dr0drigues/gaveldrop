# Testing a shell function

The shell is the only one of the six supported technologies where the subject is a **function**
rather than an executable. Everywhere else, the thing under test is a process you start; here it is
a definition you have to load first.

A case declares three things:

```yaml
name: kube-config-shows-a-resolved-path
weight: 8
setup:
  shell: zsh
  source:
    - "functions/variables.zsh"
    - "functions/ui.zsh"
    - "functions/kube_config.zsh"
  call: ["kube_config_show"]
expect:
  exit_code: 0
  stdout:
    contains: [".zsh_env/scripts/fmt.zsh"]
    absent: ["$ZSH_ENV_DIR"]
```

`shell:` is the interpreter — `bash` or `zsh`. `source:` lists the files to load. `call:` is the
function and its arguments.

None of those three words exist in gaveldrop's core. They travel through `setup`, whose keys are
opaque past `run` and `exec` by design, and only the shell adapter reads them. That is also why a
case with `shell:` is routed to that adapter without any configuration saying so.

## `source:` is a list, and the order is the point

The list covers both ways to load a shell project, and the format has no opinion on which you want:

- `["rc.zsh"]` — the **full load**, entry point and all. Faithful, and expensive: a real `rc.zsh`
  initialises completions, clones plugins over the network and calls out to starship, mise, zoxide
  and direnv. Inside isolation, most of that is missing, and what remains you pay for on every case.
- `["functions/ui.zsh", "functions/kube_config.zsh"]` — the **selective load**. Fast, deterministic,
  and what you want almost always.

The files are sourced left to right, because loading order is load-bearing in a real shell project:
a function file that uses the UI library has to come after it. A bug in that order is only visible
when the order runs, which is why the selective load is a choice rather than the rule.

## Paths point at your repository

A relative `source:` resolves against the **project root**, not the isolated directory. A case
naming `functions/ui.zsh` means the file in your repository, because that file *is* the subject.

Reading a project file is not a hole in the isolation. Writing would be, and nothing permits it: the
function still runs with the isolated root as its working directory and the isolated `HOME`, so
everything it creates lands inside. An absolute path is left alone, since someone naming one meant
it.

## Arguments are inert

Every word in `call:` is single-quoted before it reaches the shell. An argument containing `;`,
`$(…)` or a newline arrives as one argument:

```yaml
call: ["greet", "hi; rm -rf /"]
```

The function receives that string. A case file is data, not a script, and it stays data even when
someone writes something that looks like one.

## A module guarded by a flag

Shell configuration is full of files that do nothing unless a variable says so. `setup.env` is how a
case turns one on, and how it tells the file where the repository is:

```yaml
setup:
  shell: zsh
  source: ["modules/tools/posting/init.zsh"]
  call: ["true"]
  env:
    ZANVIL_MODULE_POSTING: "true"
    ZANVIL_DIR: "$GAVELDROP_PROJECT"
expect:
  exit_code: 0
  stdout:
    contains: ["brew install posting"]
```

`call: ["true"]` because the point is the **sourcing**: the file's top-level code runs, and what it
printed or defined is what the case asserts on. Nothing in the module changes to make this possible,
which is the second property.

The case above has a problem, and it is the interesting one. `PATH` inside the isolation is the
directory of fake symlinks followed by the **inherited** one — isolation has to keep `sh` and the rest
working. So `command -v posting` finds the real tool on a machine that has it, and the same case
passes on a bare CI runner while failing on a laptop where you installed it.

Faking does not help: a fake is a symlink, so it makes the tool *present*. `hide:` is the other half.

```yaml
setup:
  shell: zsh
  source: ["modules/tools/posting/init.zsh"]
  call: ["true"]
  hide: [posting]
  env:
    ZANVIL_MODULE_POSTING: "true"
    ZANVIL_DIR: "$GAVELDROP_PROJECT"
expect:
  exit_code: 0
  stdout:
    contains: ["brew install posting"]
```

Now the verdict is the same everywhere, which is the whole point of running in isolation.

**It removes whole directories, and you have to know that.** `PATH` has no finer granularity: a shell
walks the directories and reports the first hit, so making one name unfindable means dropping every
directory that holds it. Hiding `posting` drops `/opt/homebrew/bin`, and anything installed only
there goes with it. A case that then needs one of those fails with a command not found — loud, which
is the requirement, but surprising the first time.

Naming a tool your project also fakes is **fine, and the case wins** — no symlink is laid down for
it. That is what lets one configuration hold both branches of a guarded module: the project fakes the
tool so most cases exercise the "present" path, and the case that proves the warning hides it.

```yaml
# gaveldrop.yaml — the whole suite fakes it
fake:
  bins: [posting]
```

```yaml
# and this one case does not want it found
setup:
  hide: [posting]
```

## A filter, reading its input from the case

`stdin` in, `stdout` out is the commonest shape a terminal tool takes, and the case carries the input:

```yaml
name: a-malformed-line-is-passed-through-unchanged
weight: 3
setup:
  stdin: |
    {"level":"INFO","message":"ready"}
    {"level":"INFO","message":"missing brace"
  run: ["$GAVELDROP_PROJECT/scripts/format-logs.sh"]
expect:
  exit_code: 0
  stdout:
    contains: ['{"level":"INFO","message":"missing brace"']
```

Written in the case rather than read from a fixture file, and that is a choice: YAML's `|` carries as
many lines as you like, and a case holding both its input and its expectation reads in one piece
instead of sending you elsewhere for half the story. When an input is too big to sit in a case, that is
worth noticing rather than working around.

**The input is data, not a template.** `run` substitutes variables because it is a command line, and
`env` because it is configuration — `stdin` is neither. A log line may legitimately contain `$HOME`,
and expanding it would corrupt the very thing under test.

A subject that stops reading early is fine: `head -1` closes the pipe once it has what it wants, and
that is its business rather than a failure. Large inputs are fine too — the input goes out on its own
thread, so a filter over more than a pipe's worth of data cannot deadlock against its own output.

It works for `run:` and for `call:` alike. It does **not** apply to `serve:`, where the subject is a
service gaveldrop starts and polls: a service reading its standard input is not the shape that adapter
is for.

## Dependencies are faked the same way

A function calling `kubectl` is intercepted exactly as a binary would be, because the fake is an
executable placed first on `PATH` rather than a library:

```yaml
setup:
  shell: zsh
  source: ["functions/kube_config.zsh"]
  call: ["kube_config_switch", "staging"]
fake:
  rules:
    - match: { bin: kubectl, args_contain: "config use-context" }
      respond: { stdout: "Switched to context staging", exit: 0 }
expect:
  calls:
    kubectl: 1
```

Nothing about faking is shell-specific. The engine, the journal and the four response modes are the
same ones a Rust or Python case uses.

## Files written outside the repository

This is where a shell project usually hurts: a function that drops a config in `~/.kube/` or
`~/.config/`. Since `HOME` is the isolated root, those writes land inside it and are observed
without any extra machinery:

```yaml
expect:
  files:
    "$HOME/.kube/config":
      contains: ["current-context: staging"]
```

Paths may use the variables isolation defines — `$HOME`, `$XDG_CONFIG_HOME` and the rest — or a
leading `~`. Anything else is refused rather than left literal, because a stray `$TYPO` would make
an `absent` assertion trivially true.

## The shell has to be installed

There is no skipping. A case declaring `shell: zsh` on a machine without zsh **fails**, with:

```
starting `zsh`: No such file or directory (os error 2)
```

A skipped case reads as coverage while proving nothing, and that is a failure mode this project has
already had to remove from its own conformance kit. The message names what to install.

Note for CI: `zsh` is **not** on GitHub's `ubuntu-24.04` image — bash, dash and PowerShell are all it
ships — while macOS has had it as the default shell for years. This repository's Linux job installs
it explicitly.

## What is not here

`idempotent: true` does not exist. Shell configuration functions that append to `PATH` are a classic
place for a second call to do damage, so the property is real — but a boolean would hide that the
subject runs twice and say nothing about what is compared: output? files? both? Diagnosing its
failure would mean reading gaveldrop, which is exactly what the project's third property forbids.

It lands with multi-step cases, as two visible invocations plus an expectation comparing them.
Longer to write, and honest about the cost.

`capture:` does not work here either. It is part of every step, because steps belong to the format
rather than to one adapter — but naming a value from a response by JSON path assumes a response with
a structure, and a shell function answers text. Deciding that its output is a JSON document to walk
would be inventing a meaning for the format rather than implementing one.

So a case declaring one is **told**, at the place it declared it:

```
FAIL a-function-reads-back-what-it-wrote  0/3
    steps[0] "writes it".capture.order_id
      expected  a value at data.order.id
      got       the path led nowhere, so `$order_id` stays literal in every later request.
                The body was empty
```

Which is the honest answer, if not a satisfying one: the path found nothing because there was
nothing to look in. It used to be silence — nothing captured, nothing said, and `$order_id` literal
in the next call.
