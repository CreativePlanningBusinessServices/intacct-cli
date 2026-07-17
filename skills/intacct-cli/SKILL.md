---
name: intacct-cli
description: Use when a task needs Sage Intacct data or metadata from the command line — reading or writing objects, running queries, discovering schemas, bulk/composite operations, or any Intacct REST API call. Covers picking the right subcommand, bootstrapping an account, recipes, and triaging errors by exit code.
---

# intacct-cli

Single-binary CLI for the Sage Intacct REST API. stdout is always exactly one JSON
value; errors are JSON on stderr. Add `--pretty` for humans.

## Pick the right command

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

Object paths are `application/object` (`accounts-payable/vendor`, `general-ledger/journal-entry`),
document types are `application/document::Name` (quote it), custom objects are
`platform-apps/nsp::name`.

## Bootstrap (once per machine)

```bash
export INTACCT_CLI_CLIENT_SECRET='<secret>'   # or omit to be prompted interactively
intacct-cli account add prod --company-id <companyId> --flow client-credentials \
  --client-id <clientId> --user-id <webServicesUserId>
intacct-cli account test        # verifies credentials end-to-end
```

First account added becomes the default; select others with `--account <alias>` or
`$INTACCT_ACCOUNT`. Prereqs (admin, once per company): a Web Services user, and the
app's client ID authorized under Company > Setup > Company > Security > Authorized
Client Applications.

## Recipes

- **Discover before writing.** `intacct-cli describe accounts-payable/bill --schema` shows
  required fields, enum values, `isReadOnly` (never send those), and owned-object shapes.
- **Query paging:** `--size` max 4000; `--all` merges every page into
  `{items, count, totalCount, hasMore}`.
- **Filters** are JSON, repeatable, combined with `--filter-expression "(1 and 2) or 3"`:
  `--filter '{"$eq":{"status":"active"}}' --filter '{"$contains":{"name":"Acme"}}'`
  Operators: $eq $ne $lt $lte $gt $gte $in $notIn $between $notBetween $contains
  $notContains $startsWith $notStartsWith $endsWith $notEndsWith.
- **Aggregates** go in --fields: `--fields 'count:key,sum:totalDue'`.
- **Batch writes:** pass a JSON array to `object create`/`update` (max 500; add `--atomic`
  for all-or-nothing). Batch update items each carry their own `key`.
- **Idempotent writes:** `--idempotency-key <uuid>` on create/update (POST/PATCH only).
- **Multi-entity:** `--entity <entityId>` on any command scopes the request via the
  X-IA-API-Param-Entity header.
- **Owned objects** (bill lines etc.) are written through their owner in one atomic call —
  include the `lines` array in the bill body.

## Errors: exit code → action

| Exit | kind | Action |
| --- | --- | --- |
| 1 | api | Read `message` + `details[]`; quote `supportId` when contacting Sage support. 422 = business-logic validation; fix the payload. |
| 2 | usage | Bad flags/arguments — fix the invocation. |
| 3 | auth | Token/credential problem. `intacct-cli account test`; re-add the account if refresh is lost. |
| 4 | network | Transport failure after retries — check connectivity and retry. |

## Gotchas

- stdout is ALWAYS one JSON value; parse it, never scrape stderr.
- Success bodies keep Intacct's envelopes: results under `ia::result`, paging under `ia::meta`.
- `account revoke` kills ALL tokens for that user/company (not just this CLI's) — use only
  for real security events.
- Request bodies max 1 MB; batch ops max 500; query pages max 4000 rows.
- The tenant lives in the OAuth username (`user@company|entity`), not the URL — there is
  one global API host.
- GET list endpoints return key/id/href-level rows only; use `query` for real field access.
- Schedules (`core/schedule`, `core/scheduled-operation`), user views (`core/user-view`), and
  report definitions (`reports/interactive-custom-report`) are plain objects — manage them
  with `object` CRUD. `view run` executes views; `report run` regenerates stored reports.
