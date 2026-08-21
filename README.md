# intacct-cli

A single-binary Rust CLI wrapping the Sage Intacct REST API for AI agents and
scripting: multi-account OAuth (client-credentials and authorization-code flows),
object CRUD, filtered queries, schema discovery, async bulk jobs, composite
requests, saved views, exports, and stored reports. stdout is always exactly one
JSON value (errors are JSON on stderr too), so every command is pipeable to `jq`
without scraping human-readable text. Modeled on
[`netsuite-cli`](https://github.com/CreativePlanningBusinessServices/netsuite-cli).

## Install

**Release zip:** download the archive for your platform from the
[latest release](../../releases/latest), unzip, and put `intacct-cli` on your
`PATH`. Prebuilt for `aarch64-apple-darwin`, `x86_64-apple-darwin`, and
`x86_64-pc-windows-msvc`.

**From source:**

```bash
cargo install --path .
```

Linux builds have no OS-keyring backend wired up (keyring falls back to an
in-memory mock, so stored credentials do not persist) — macOS and Windows are
the supported platforms; Linux is CI-only.

## Bootstrap (once per machine)

```bash
export INTACCT_CLI_CLIENT_SECRET='<secret>'   # or omit to be prompted interactively
intacct-cli account add prod --company-id <companyId> --flow client-credentials \
  --client-id <clientId> --user-id <webServicesUserId>
intacct-cli account test        # verifies credentials end-to-end
```

The first account added becomes the default; select others with `--account <alias>`
or `$INTACCT_ACCOUNT`. Prerequisites (admin, once per company): a Web Services
user, and the app's client ID authorized under Company > Setup > Company >
Security > Authorized Client Applications.

For the auth-code flow (`--flow auth-code`), the Sage app registration must list
`https://127.0.0.1/callback` as a redirect URI. That exact form matters: Sage's
console rejects `localhost` entirely and rejects ports on IP hosts, so the CLI
listens on 443 by default and sends the portless URI (`--port` overrides both,
but then the registered URI needs the port too — which Sage currently refuses).
The login redirect hits a local listener with a self-signed certificate, so the
browser shows a warning — proceed through it. If something on your machine
already occupies port 443, pass `--paste` instead: no listener is started, and
you paste the browser's final redirect URL back into the CLI.

## Commands

| You need | Command |
| --- | --- |
| One object by key (or comma-list) | `intacct-cli object get <application/object> <key[,key...]>` |
| Filtered/field-selected reads (the workhorse) | `intacct-cli query <application/object> --fields ... [--filter ...]` |
| Cheap key/id listing | `intacct-cli object list <application/object> [--all]` |
| Create / update / delete | `intacct-cli object create|update|delete ...` |
| An object's fields, types, relationships | `intacct-cli describe <application/object>` |
| What objects/services exist | `intacct-cli describe --list [--type object|service|workflow] [--filter REGEX]` |
| >500 records in one shot | `intacct-cli job submit ...` (async bulk) |
| Several calls in one round trip | `intacct-cli composite --data '[...]'` |
| Run a saved system/user view | `intacct-cli view run ...` |
| Export filtered data | `intacct-cli export ...` |
| Anything without a dedicated command | `intacct-cli raw <METHOD> <path> ...` |

Object paths are `application/object` (`accounts-payable/vendor`,
`general-ledger/journal-entry`), document types are
`application/document::Name` (quote it), custom objects are
`platform-apps/nsp::name`.

See `intacct-cli skill install` and [`skills/intacct-cli/SKILL.md`](skills/intacct-cli/SKILL.md)
for the full agent-facing usage guide (query filters, batch writes, idempotency
keys, multi-entity requests, owned objects, and gotchas), and
[`docs/superpowers/specs/2026-07-16-intacct-cli-design.html`](docs/superpowers/specs/2026-07-16-intacct-cli-design.html)
for the design spec.

## Exit codes

| Exit | kind | Meaning |
| --- | --- | --- |
| 1 | api | Sage Intacct returned an error. Read `message` + `details[]`; quote `supportId` when contacting Sage support. 422 = business-logic validation. |
| 2 | usage | Bad flags/arguments — fix the invocation. |
| 3 | auth | Token/credential problem. Run `intacct-cli account test`; re-add the account if refresh is lost. |
| 4 | network | Transport failure after retries — check connectivity and retry. |

## Development

```bash
cargo test --locked
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

### Live smoke tests

`tests/live_smoke.rs` exercises a configured account against the real Sage
Intacct API. It's excluded from `cargo test` by default (`#[ignore]`) and from
CI; run it manually against an account already added via `account add`:

```bash
INTACCT_LIVE_ALIAS=<alias> cargo test --test live_smoke -- --ignored --nocapture
```

### Release procedure

Tag and push to trigger the release workflow, which builds, zips, and publishes
binaries for macOS (arm64/x86_64) and Windows (x86_64):

```bash
git tag vX.Y.Z && git push --tags
```

## Task

Basecamp: [Intacct CLI](https://basecamp.com/2808802/projects/8218129/todos/518814675) (todo 518814675, ERP Internal Projects)
