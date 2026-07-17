# intacct-cli

A Rust CLI wrapping the Sage Intacct APIs, modeled on [`netsuite-cli`](https://github.com/CreativePlanningBusinessServices/netsuite-cli).

## Scope

Wrap as much of the API surface documented at <https://developer.intacct.com/> as practical.

**In scope:** the modern REST API.

**Out of scope:** the legacy XML APIs.

## Status

Scaffold only — no implementation yet. `src/main.rs` is the stock `cargo new` hello-world.

## Reference

`netsuite-cli` is the design reference for conventions to mirror. Its module layout:

| Module | Purpose |
| --- | --- |
| `cli.rs` | Command/arg parsing |
| `client.rs` | HTTP client |
| `config.rs` | Config file handling |
| `context.rs` | Resolved account/session context |
| `error.rs` | Error types and exit codes |
| `output.rs` | Result formatting |
| `secrets.rs` | Credential storage |
| `account.rs` | Multi-account registration |
| `auth/` | Auth flows (per-mechanism modules) |
| `commands/` | One module per subcommand |

It also ships a `skills/` directory with an agent-facing usage skill, plus `docs/` and `tests/`.

## Task

Basecamp: [Intacct CLI](https://basecamp.com/2808802/projects/8218129/todos/518814675) (todo 518814675, ERP Internal Projects)
