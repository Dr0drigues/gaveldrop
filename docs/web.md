# Testing a living service

Every other subject in gaveldrop runs to completion. A service answers requests until something stops
it, and that difference is what this adapter exists for.

```yaml
name: a-service-answers-across-steps
weight: 8
setup:
  serve: ["python3", "$GAVELDROP_PROJECT/app/server.py"]
  ready: "http://127.0.0.1:$GAVELDROP_PORT/health"
steps:
  - name: reports itself healthy
    request: { path: /health }
    expect:
      status: 200
      body:
        contains: ["\"status\": \"ok\""]
      headers:
        Content-Type: { contains: ["json"] }
  - name: creates an order
    request:
      method: POST
      path: /orders
      body: { item: chair }
    expect:
      status: 201
      body:
        contains: ["\"id\": 7"]
```

`serve:` starts the service. `ready:` is the URL to poll until it answers. `steps:` are the exchanges,
in order, each with its own expectations.

## The port is chosen for you

A suite must not fight another for a port, so gaveldrop reserves one per case and puts it in
`$GAVELDROP_PORT`. Your service reads it however it likes — an argument, an environment variable, a
config file written by a `setup.exec` hook.

`$GAVELDROP_PROJECT` is the repository root, absolute. You need it because the subject runs with the
isolated directory as its working directory, so `app/server.py` alone would point inside the
isolation, where your project does not exist.

Both are part of the closed set of variables isolation defines — the same set a path in
`expect.files` may use, and nothing from the environment of whoever runs the suite.

## What "ready" means

Any answer counts, **including a 404**. A service that replies is a service that is listening, and
demanding a 2xx would force every project to add a health endpoint before it could be tested.

Leave `ready:` out and a TCP connection to the port is used instead. That is weaker — a service can
accept connections before it can serve them — so naming a probe is usually worth the line.

Probes go out every 250 ms and each waits up to 2 seconds. Not faster, deliberately: a
single-threaded server — Python's `http.server`, a Flask development server — answers one connection
at a time with a small accept backlog, and probing faster than it can answer fills that backlog with
connections that are abandoned before their turn. The wait would then fail against a service that was
listening the whole time.

**A bound port is not a port being served.** Between binding and the accept loop, a server can spend
real time: Python's `HTTPServer` does a reverse DNS lookup while binding, which stalls for tens of
seconds where no resolver answers. So a service that logged "listening" may still not be accepting,
which is why the wait polls for an answer rather than trusting a port to be open.

A service that never answers fails its case after 30 seconds, with **both** of its streams in the
message:

```
the service was not ready after 30s (probing http://127.0.0.1:54321/health).
It wrote nothing on standard output and "Address already in use" on standard error
```

Both streams, and that matters. Stderr usually carries the reason. But its being empty is not proof
the service never started — a service that logged `listening on 54321` on stdout and still failed
this wait tells you the problem is between you and it, not in it. Reporting only stderr cost a CI
cycle to learn that.

## Steps, and what a failure names

An assertion inside a step carries the index and the name:

```
steps[1] "creates an order".status
  expected  201
  got       200
```

Naming a step is optional; locating it is not. Without a name you get `steps[1].status`, which still
beats counting lines in the document.

A mismatch between declared and performed exchanges is a failure **in both directions**. Fewer means
the subject stopped halfway, and comparing only what came back would report green. More means an
exchange happened that the case never declared.

## Writing a request

Everything is optional. A step with no `request:` performs `GET /`, which is what a smoke exchange
looks like.

- `method:` — `GET` when unstated, and case-insensitive.
- `path:` — a path, not a full URL, because the port is chosen per run. A missing leading slash is
  added for you.
- `body:` — written as a mapping it is sent as JSON, so a case testing a JSON API does not escape
  quotes inside a YAML string. Written as a string it is sent verbatim, for a form or plain text.
- `headers:` — a name-to-value map.

Header names are compared case-insensitively, so asserting `Content-Type` against a server sending
`content-type` works. A missing header reports what the response *did* carry, since the cause is
usually a typo in the name.

## `expect` and `steps[].expect`

The top-level `expect` describes the run **as a whole** — for a service that is its own logging and
the files it wrote, which belong to no single exchange:

```yaml
expect:
  stdout:
    contains: ["listening on"]
  files:
    "$HOME/orders.log":
      contains: ["created chair"]
```

`exit_code` is worth a note: with exchanges declared it is always `0`, because the subject is still
running while they happen. Reporting the code it will eventually be *killed* with would be asserting
on how gaveldrop stops it, which is not a property of your service.

## Faking the APIs your service calls

The rule engine has a second door. Same rules, same journal, same catch-all — a request on a port
instead of an executable on `PATH`:

```yaml
fake:
  rules:
    - match: { bin: /products }
      status: 200
      stdout: '[{"sku":"CHR-1","name":"chair"}]'
      headers:
        Content-Type: application/json
    - match: {}
      status: 503
      stdout: '{"error":"the case did not foresee this call"}'
expect:
  calls:
    /products: 1
```

The faked service listens on `$GAVELDROP_FAKE_PORT`, which your service reads the same way it reads
its own port. How the request maps onto the rules:

| Request | Matches against | So you can write |
|---|---|---|
| path | `bin` | `match: { bin: /products }` |
| method and query | `args_contain` | `match: { args_contain: POST }` |
| body | `stdin_contains` | `match: { stdin_contains: '"urgent":true' }` |

The counter key is the path, so `call: 2` means the second request to it — which is how you fake a
retry answering differently the second time.

Two modes this door does not honour, refused when the service starts rather than at the first
request: `exec: real`, because there is no next service along a port, and `render:`, which would need
a hook's output captured. Both name themselves in the error.

## The two accepted gaps

**The port race.** A reserved port is released before your service binds it, so something else on the
machine could take it in between. The alternative — handing an already-open socket to a child — only
works for a subject we compile ourselves, which is the assumption this project exists to refuse.

**The stop is not graceful.** `SIGKILL`, so your shutdown handler does not run and "shutting down"
never reaches the log. What is lost is only what the service writes *during* shutdown: both streams
are drained continuously while it runs, so everything before that is observed.

## What is not here

`idempotent:` does not exist yet, and neither does carrying a value from one step into the next — an
id created by step 1 used as a path in step 2. That is the point where a case format starts becoming a
programming language, and the line is worth drawing once real cases have shown what they actually
need. Both land in lot 6b; see `ROADMAP.md`.
