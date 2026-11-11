# Stellar Tags

Stellar Tags is a payment platform that combines a Soroban smart contract, a Node.js server, and a React dashboard. It is structured as a small mono-repo so each piece can be developed and deployed independently while still working together as a single product.

## What is inside

- `payment-dashboard/` - React + Vite frontend dashboard.
- `stellar-payment-platform/` - Node.js server for API and business logic.
- `payment_router/` - Rust/Soroban contract.

## Key features

- Desired specific username
- Fast transfer
- Secured payment flows

## Architecture Map

The following diagram maps exactly how data flows between the user, Render, and the Stellar network.

```text
[ User / Browser ]
       |
       | (Vite App hosted on Vercel)
       v
[ payment-dashboard ]
  (src/App.jsx: Wallet connections & UI)
       |
       | HTTP API Calls (via VITE_API_BASE)
       v
[ stellar-payment-platform ] <---> [ PostgreSQL Database ]
  (server.js: Server router on Render)        (via Prisma ORM: User/payment layout)
       |
       | Stellar Network / RPC
       v
[ payment_router ]
  (src/lib.rs: Soroban smart contract routing logic)
```

**Data Flow:**
1. **User** accesses the `payment-dashboard` and connects their Stellar wallet.
2. The dashboard queries the `stellar-payment-platform` server for user registrations and payment routing information.
3. The server interacts with its PostgreSQL database (via the Prisma ORM) to resolve usernames to addresses using the endpoints documented below.
4. When a payment is initiated, it's routed through the `payment_router` Soroban contract on the Stellar network.

## Repository structure

```text
.
├── payment-dashboard/
│   ├── .env                 # Frontend environment variables
│   └── src/
│       └── App.jsx          # Wallet connections and React UI
├── payment_router/
│   └── src/
│       └── lib.rs           # Soroban smart contract logic
└── stellar-payment-platform/
    ├── server.js            # Server router (Express API endpoints)
    └── prisma/
        └── schema.prisma    # Prisma schema for the PostgreSQL database
```

## Getting started

> These steps are split by module so you can run only what you need.

### Docker Compose profiles

Every service in `docker-compose.yml` belongs to a profile, so
`docker compose up` on its own starts nothing and you always say which stack
you want:

| Command | Starts |
| --- | --- |
| `docker compose --profile dev up` | backend, postgres, redis |
| `docker compose --profile full up` | the same, plus the built frontend on :3000 |
| `docker compose --profile test up` | the API under test on :5001 and its own database |
| `docker compose --profile integration up` | a local standalone Stellar network on :8000 |

`dev` is the everyday one. `full` adds the frontend, which is the slowest
thing in the file to build and is not needed for backend work.

The dev and test stacks use separate databases on separate host ports (5432
and 5433), so you can run both at once:

```bash
docker compose --profile dev --profile test up -d
```

`COMPOSE_PROFILES` works too, if you would rather not repeat the flag:

```bash
export COMPOSE_PROFILES=dev
docker compose up -d
```

Stop a stack with the same profile you started it with, otherwise Compose
will not know which services it is meant to remove:

```bash
docker compose --profile dev down
```

The `dev` PostgreSQL service stores its data in the named volume
`postgres_dev_data` (mounted at `/var/lib/postgresql/data`), so rows survive
container restarts and `down`/`up` cycles — no need to re-run
`npm run db:seed` every session. The `test` profile database deliberately has
no volume, so each test run starts from a clean slate.

To wipe the dev database and start fresh, remove the volume while the stack is
stopped:

```bash
docker compose --profile dev down
docker volume rm postgres_dev_data
docker compose --profile dev up -d
```

The `integration` profile pulls `stellar/quickstart`, a multi-gigabyte image
used only by the Soroban contract tests in `tests/integration/`. It is kept
out of `test` so the API tests do not drag it in.

### Frontend dashboard

```bash
cd payment-dashboard
npm install
npm run dev
```

### Server

The server uses **PostgreSQL** as its database, accessed through the
[Prisma ORM](https://www.prisma.io/). You need a running Postgres instance
(local install, Docker, or a hosted provider) before starting the server.

```bash
cd stellar-payment-platform
npm install

# 1. Create your local env file and point DATABASE_URL at your Postgres DB
cp .env.example .env
#    then edit .env (see "Database setup" below)

# 2. Apply the schema to your database
npm run prisma:migrate

# 3. Fill the database with test data
npm run db:seed

# 4. Start the server
npm run dev
```

#### Database setup

The connection string lives in `stellar-payment-platform/.env` as `DATABASE_URL`.
Copy `.env.example` to `.env` and set it to your own Postgres database:

```env
DATABASE_URL="postgresql://USER:PASSWORD@HOST:PORT/DATABASE?schema=public"
```

For a typical local install that becomes, for example:

```env
DATABASE_URL="postgresql://postgres:postgres@localhost:5432/stellar_tags?schema=public"
```

The server appends Prisma connection-pool limits to `DATABASE_URL` at startup
(`src/db.js`), so a constrained host cannot open more connections than
PostgreSQL allows. Tune them with:

- `DB_POOL_SIZE` - (Optional) Maximum connections in the Prisma pool. Defaults to `10`.
- `DB_POOL_TIMEOUT` - (Optional) Seconds a query waits for a free connection before failing. Defaults to `30`.

For example, `DB_POOL_SIZE=5` limits the pool to 5 connections. If
`DATABASE_URL` already contains `connection_limit` or `pool_timeout`, those
values are left untouched.

The quickest way to get a local database is the dev profile, which also
brings up Redis:

```bash
docker compose --profile dev up -d postgres redis
```

Or a single container, if you want nothing else:

```bash
docker run --name stellar-postgres -e POSTGRES_PASSWORD=postgres \
  -e POSTGRES_DB=stellar_tags -p 5432:5432 -d postgres:16
```

Useful Prisma commands (run from `stellar-payment-platform/`):

| Command | Description |
| --- | --- |
| `npm run prisma:migrate` | Create/apply migrations against your dev database |
| `npm run prisma:deploy` | Apply existing migrations (CI / production) |
| `npm run prisma:generate` | Regenerate the Prisma Client after schema changes |
| `npm run prisma:studio` | Open Prisma Studio to browse the data |
| `npm run db:seed` | Populate the database with test data (see below) |

> `.env` is gitignored — never commit real credentials. Each contributor keeps
> their own local `DATABASE_URL`.

#### Seed data

`npm run db:seed` populates an empty database with 50 users, 10 webhooks,
20 payment intents and 2 API keys, so the API has something to answer with
before you have registered anything by hand. `prisma migrate reset` runs it
automatically.

The data covers the cases the endpoints branch on: usernames with every memo
type, several addresses carrying aliases alongside their primary username,
flagged and soft-deleted users, webhooks that are healthy, never delivered,
and failing on both sides of the retry cutoff, and payment intents in each
status.

Re-running is safe. Every record is upserted on its natural key and the
identities are derived from a fixed seed, so a second run updates the same
rows rather than adding a set. Pass `--reset` to delete the seeded rows first:

```bash
npm run db:seed -- --reset
```

Both API keys are printed on each run, since only their hashes are stored.
Use the active one as `X-Api-Key`; the revoked one is there to exercise the
rejection path. The script refuses to run against a non-local `DATABASE_URL`
unless `SEED_ALLOW_REMOTE=1` is set.

### Render deployment

The repository includes a [render.yaml](render.yaml) blueprint for the backend API and its PostgreSQL database. When you deploy from Render, import the blueprint or create the service from the repo so `DATABASE_URL` is injected automatically from the managed database.

The blueprint sets `MIGRATION_POLICY=strict` for the production service. Under the default `warn` policy a deploy with pending migrations would still bind a port and serve traffic against an out-of-sync schema, which can cause silent data corruption or query failures. With `strict`, the startup migration check logs an error and exits non-zero **before** `app.listen()`, so the deploy fails its health check and Render never routes traffic to an instance whose schema is not fully migrated. Schema drift becomes a deployment blocker instead of a runtime surprise.

If you deploy the backend without the blueprint, make sure the web service has a PostgreSQL `DATABASE_URL` secret configured before startup, and set `MIGRATION_POLICY=strict` in the service environment to get the same protection. The container runs Prisma migrations on boot, so the variable must already exist.

### Smart contract (Soroban)

```bash
cd payment_router
cargo build
```

#### Daily spending limit

The contract enforces a per-user daily spending limit. The window is a
**24-hour wall-clock window (86,400 seconds)** measured from the ledger
timestamp (`env.ledger().timestamp()`), not from a ledger sequence count.
The timestamp of the last reset is stored in persistent storage alongside the
daily spend, so:

- a payment made 86,399 seconds after the last reset still counts against the
  current window;
- a payment made 86,401 seconds after the last reset starts a fresh window and
  resets the counter.

Because the window is time-based, it no longer drifts with ledger close times
(which average ~5 seconds but are not guaranteed), so the limit resets when
users expect it to.

### Contract TypeScript bindings

The TypeScript client for the `payment_router` contract lives in
[`packages/types`](packages/types) and is **auto-generated** from the contract
ABI, so the React dashboard gets end-to-end type safety with the Rust contract
instead of manually copying contract IDs and argument shapes.

Regenerate the bindings after any change to the contract's public interface
(the contract must build with the `wasm32-unknown-unknown` target, and the
`stellar` CLI must be on your PATH):

```bash
npm run generate:bindings
```

The result is committed to `packages/types` and consumed by the frontend as
`@stellar-tags/payment-router` (a `file:` dependency). The `bindings-check` CI
job fails the build if the checked-in bindings ever drift from the contract
ABI.

The generated `index.ts` exposes each contract method as an **individual named
export** rather than a single class instance, so bundlers can tree-shake
contract methods the dashboard never calls. Import only what you use:

```ts
import { get_refund_balance } from '@stellar-tags/payment-router';
```

Importing one method no longer pulls the rest of the binding module into the
Vite bundle.

## Webhook signature verification

Every webhook delivery includes an HMAC-SHA256 signature in the
`X-Webhook-Signature` header (and the backward-compatible alias
`X-Stellar-Tags-Signature`). Merchants must verify this signature before
trusting the payload.

See [docs/webhook-signature-verification.md](docs/webhook-signature-verification.md)
for step-by-step verification examples in Node.js, Python, and Go.

## Tests

```bash
# frontend
cd payment-dashboard
npm test

# server
cd ../stellar-payment-platform
npm test

# contract
cd ../payment_router
cargo test
```

### Coverage

The server runs with coverage in CI via `npm run test:coverage` (same as
`npm test` plus `--coverage`). Minimum coverage thresholds are set in
`stellar-payment-platform/package.json` under `jest.coverageThreshold` — CI
fails the build when coverage drops below the floor, and the `lcov`/HTML report
is uploaded as a build artifact. Raise the thresholds as coverage improves so
the floor keeps pace with the suite.


## Environment variables

To ensure a seamless local developer installation requiring zero guesswork, please configure the following environment variables in their respective directories:

### Frontend (`payment-dashboard/.env`)
- `VITE_API_BASE` - The base URL where the frontend expects the Node.js server API to be running (e.g., `http://localhost:5000`).

### Server (`stellar-payment-platform/.env` or exported directly)
- `DATABASE_URL` - **(Required)** PostgreSQL connection string used by Prisma (see [Database setup](#database-setup)).
- `DB_POOL_SIZE` - (Optional) Maximum number of connections in the Prisma connection pool, applied as `connection_limit` on `DATABASE_URL`. Defaults to `10`.
- `DB_POOL_TIMEOUT` - (Optional) Seconds a query waits for a free pooled connection before failing, applied as `pool_timeout` on `DATABASE_URL`. Defaults to `30`.
- `PORT` - (Optional) The port for the Node.js server to listen on. Defaults to `5000`.
- `HORIZON_NETWORK` - (Optional) Stellar network for the payment listener: `testnet` (default) or `public`.
- `STELLAR_TAG_DOMAIN` - (Optional) Extra origin to add to the CORS allow-list.
- `LOG_DIR` - (Optional) Directory for the rotating log files. Defaults to `stellar-payment-platform/logs`.
- `LOG_LEVEL` - (Optional) Minimum level to record. Defaults to `info` in production and `debug` elsewhere.
- `LOG_MAX_SIZE` - (Optional) Size at which the active log file rotates. Defaults to `20m`.
- `LOG_MAX_FILES` - (Optional) Retention for rotated files, as a count (`30`) or an age (`14d`). Defaults to `14d`.
- `MIGRATION_POLICY` - (Optional) What to do at startup when `prisma migrate status` reports the database is out of sync (pending migrations or drift). Supported values:
  - `warn` (default) — logs a clear warning and continues booting. Safe for local development with mock databases.
  - `strict` — logs an error and calls `process.exit(1)` **before** `app.listen()` binds a port, so an orchestrator (Railway, Render, Kubernetes) sees a non-zero exit code and can restart a correctly-migrated instance. No port is bound and no HTTP requests are accepted while migrations are pending.
  - `off` — skips the check entirely.

For Render deployments, make sure the web service has `DATABASE_URL` set in its environment or linked from a Render PostgreSQL instance before startup. The container runs `prisma migrate deploy` during boot, so the variable must be available at runtime.

### CI / GitHub Actions Secrets
- `TESTNET_API_URL` - **(Required for `deploy-testnet.yml`)** Base URL of the deployed testnet API service (e.g., `https://api.example.com`). Used by the post-deploy health check step (`${TESTNET_API_URL}/health`) to verify an HTTP `200` response and `status: "up"` before marking the deployment workflow green.

## Logging

The server logs through a shared [Winston](https://github.com/winstonjs/winston) logger
(`stellar-payment-platform/src/logger.js`). Import it instead of calling `console` directly:

```js
const { logger } = require('./src/logger');

logger.info('Registered tag', { correlationId: req.correlationId, tag });
logger.error(err);
```

Log records are written as JSON to `stellar-payment-platform/logs/`:

- `application-YYYY-MM-DD.log` - everything at `LOG_LEVEL` and above.
- `error-YYYY-MM-DD.log` - errors only, so incidents are easy to find.

Both files rotate **daily and whenever they pass 20MB**, older files are gzipped, and
anything beyond the retention window is deleted, so logs cannot exhaust the disk. A
human-readable copy is also printed to the console (silenced when `NODE_ENV=test`, which
also disables file output so test runs leave no logs behind).

### PII in debug logs

Stellar public keys are PII, so at `LOG_LEVEL=debug` the logger truncates any
G-address (a string matching the Stellar public key format, `G` followed by 55
uppercase alphanumerics) to its first 8 and last 4 characters before writing:

```text
GABC1234...5678
```

This keeps debug output useful for tracing without persisting full addresses to
rotating log files or a log aggregator. At `info` level and above keys are logged
in full, since those records are needed for audit purposes. The truncation is a
Winston format applied only to debug-level records; memo text and usernames are
not sanitised.

## Error responses

Every API error leaves the server in one shape, produced by a single terminal
handler (`stellar-payment-platform/src/middleware/errorHandler.js`):

```json
{
  "success": false,
  "error": {
    "code": "VALIDATION_FAILED",
    "message": "Invalid request body",
    "details": [{ "field": "username", "message": "username is required" }]
  },
  "correlation_id": "3f2a…",
  "reference_id": "9b41…"
}
```

`error.code` is the stable part of the contract — branch on it rather than on
the status or the message text, which may be reworded. `details` appears only
when the failure is field-level. `correlation_id` is on every error;
`reference_id` is added on `5xx` and matches the logged stack.

| Code | Status | Raised when |
| --- | --- | --- |
| `INVALID_INPUT` | 400 | Malformed query, JSON, or a rejected value |
| `UNAUTHENTICATED` | 401 | Missing or failed signature verification |
| `FORBIDDEN` | 403 | Reserved name, blocked address |
| `NOT_FOUND` | 404 | No such tag, address, or route |
| `METHOD_NOT_ALLOWED` | 405 | Wrong verb on a known path |
| `CONFLICT` | 409 | Username already taken, or an address is at its 5-username limit |
| `PAYLOAD_TOO_LARGE` | 413 | Body over the 10kb cap |
| `UNSUPPORTED_MEDIA_TYPE` | 415 | Non-JSON body on a JSON endpoint |
| `VALIDATION_FAILED` | 422 | Body failed its schema |
| `RATE_LIMITED` | 429 | Rate limit exhausted |
| `INTERNAL_ERROR` | 500 | Unhandled failure |
| `UPSTREAM_ERROR` | 502 | Horizon or another upstream failed |
| `SERVICE_UNAVAILABLE` | 503 | Database or Redis unreachable, request timeout |

To raise one, throw or pass an `ApiError` — the handler is the only place that
turns an error into a response:

```js
const { ApiError } = require('./src/errors');

return next(new ApiError('CONFLICT', 'Username is already taken. Please choose another.'));
```

A `5xx` from an unexpected throw always reports the generic message so
internals are never leaked; the real error goes to the log under
`reference_id`. A message passed deliberately to `ApiError` is sent as written.

`GET /health` is exempt: it reports component status (`{ status, database,
redis, horizon }`) rather than an API error.

## Request validation

Incoming request bodies and query strings are validated by
[zod](https://zod.dev) schemas before any route handler runs. Schemas live in
`stellar-payment-platform/src/schemas/index.js`, and
`src/middleware/validateSchema.js` turns them into route middleware:

```js
const { validateSchema } = require('./src/middleware/validateSchema');
const { registerBodySchema, usersQuerySchema } = require('./src/schemas');

app.post('/register', validateSchema({ body: registerBodySchema }), handler);
app.get('/users', validateSchema({ query: usersQuerySchema }), handler);
```

The validated part is replaced with the parsed result, so handlers receive
values that are already trimmed and coerced — `req.query.limit` is a number,
not a string — and never re-check types themselves.

Failures short-circuit before the handler and respond with the field-level
errors, using the status that matches where the bad input came from:

| Failure | Status | Body |
| --- | --- | --- |
| Invalid `req.body` | `422 Unprocessable Entity` | `{ success: false, errors: [{ field, message }] }` |
| Invalid `req.query` | `400 Bad Request` | `{ success: false, errors: [{ field, message }] }` |

Two rules are deliberately *not* in the schemas, because the handlers own them
and answer `400` with their own domain-specific messages: Stellar address
format (checked with `StrKey`) and memo pairing/format (checked with
`validateMemo`). `page` and `limit` clamp to their bounds rather than being
rejected, so `?limit=1000` still returns the maximum page size.

## Detailed Endpoint Documentation

The Node.js server (`stellar-payment-platform/server.js`) exposes the following endpoints for username and payment lookups:

### `GET /federation`
Resolves a given username tag to a Stellar address.
- **Query Parameter:** `q` (string) - The username tag to lookup (e.g., `alice*localhost`).
- **Returns:** A JSON object with `stellar_address`, `account_id`, `memo_type`, and `memo`.
- **Status Codes:**
  - `200 OK`: Address found.
  - `400 Bad Request`: Missing `q` parameter.
  - `404 Not Found`: Name tag not found.
  - `500 Internal Server Error`: Database lookup failed.

### `POST /register`
Registers a new username and associates it with a Stellar address. An address
may hold up to 5 usernames (aliases), e.g. `payments*domain` and
`support*domain` for one business account. The first username registered for an
address is its primary; reverse (`type=id`) federation lookups resolve to it.
- **Body Parameters (JSON):** 
  - `username` (string) - The desired username.
  - `address` (string) - The user's Stellar address.
- **Returns:** A JSON object with registration details `{ ok: true, username, address, is_primary }`.
- **Status Codes:**
  - `200 OK`: Registration successful.
  - `400 Bad Request`: Missing `username` or `address`.
  - `409 Conflict`: Username already taken, or the address already has the maximum of 5 usernames.
  - `500 Internal Server Error`: Database lookup or insertion failed.

### `GET /lookup`
Resolves a given Stellar address to its registered username. When an address has
several usernames, the primary one is returned.
- **Query Parameter:** `address` (string) - The Stellar address to lookup.
- **Returns:** A JSON object with `username` and `address`.
- **Status Codes:**
  - `200 OK`: Username found.
  - `400 Bad Request`: Missing `address` parameter.
  - `404 Not Found`: Username not found for this address.
  - `500 Internal Server Error`: Database lookup failed.
- **Performance:** `username_registry.address` is backed by a non-unique index
  (`@@index([address])` on `User`, created by the
  `20260829120000_add_username_aliases` migration), so reverse lookups stay an
  index scan instead of a full table scan as the user count grows.

### `GET /users/:username/activity`
Returns the caller's own activity trail: registrations, transfers,
unregistrations, webhook creation and deletion, and blocks applied to their
address.

Ownership is proven the same way the webhook endpoints prove it. Sign the
message `activity:<username>` with the account key and send the base64
signature:

```bash
curl "http://localhost:5000/users/ada*localhost/activity?limit=20" \
  -H "X-Stellar-Signature: <base64 signature>" \
  -H "X-Stellar-Signer: <G... public key>"
```

The signature may also be sent in the request body as `signature` /
`signerAddress`, matching `GET /webhooks`.

- **Query Parameters:**
  - `page` (optional) - 1-based page number, default 1.
  - `limit` (optional) - rows per page, default 10, capped at 100.
  - `startDate` / `endDate` (optional) - inclusive bounds on `created_at`.
- **Returns:** `{ data, meta: { total, page, limit, totalPages } }`, newest
  first. Each row carries `id`, `action`, `metadata`, `ip_address` and
  `created_at`.
- **Status Codes:**
  - `200 OK`: Trail returned.
  - `400 Bad Request`: Missing signature, or an unparseable/inverted date range.
  - `401 Unauthorized`: The signature does not belong to the account behind the
    username.
  - `404 Not Found`: Username not registered.

Actions are namespaced: `user.registered`, `user.unregistered`,
`user.transferred`, `user.blocked`, `webhook.created`, `webhook.deleted`. Rows
are removed with the user, so a purge does not leave a trail behind.

### `GET /health`
Aggregates the status of every external dependency: PostgreSQL (a `SELECT 1`
through Prisma), Redis (`PING`) and Stellar Horizon (an HTTP request to
`HORIZON_BASE`). The three probes run in parallel.
- **Returns:** `{ status, timestamp, database, redis, horizon }`, where each
  dependency is `up`, `down`, or `not configured` (Redis, when `REDIS_URL` is
  unset). A `DOWN` response also carries a `message` naming the failures.
- **Status Codes:**
  - `200 OK`: Every configured dependency responded.
  - `503 Service Unavailable`: At least one dependency is down.

`HEALTH_HORIZON_TIMEOUT_MS` (default 3000) bounds the Horizon probe so a
hanging Horizon cannot hold the response open.

### `GET /transactions/export`
Streams the account's payment history as a CSV download.
- **Query Parameters:** `address` (required) - Stellar public key. `order` (optional) - `desc` (default) or `asc`.
- **Returns:** `text/csv` with a `Content-Disposition` attachment header. Columns: `id`, `created_at`, `type`, `from`, `to`, `amount`, `asset_type`, `asset_code`, `asset_issuer`, `transaction_hash`.
- **Status Codes:**
  - `200 OK`: Stream started. Sent chunked, so there is no `Content-Length`.
  - `400 Bad Request`: Missing or invalid `address`.
  - `404 Not Found`: Account not found on Horizon.
  - `502 Bad Gateway`: Horizon request failed.

Pages of 200 records are fetched from Horizon with its cursor, converted, and
flushed as they arrive, so neither the full result set nor the full CSV is held
in memory: heap use plateaus around 18MB whether the export is 5,000 rows or
100,000. Writes respect socket backpressure, and `EXPORT_MAX_PAGES`
(default 500) bounds a single export — a truncated export is logged as a
warning. Because the response is committed once streaming starts, a mid-stream
failure can only be logged and the connection cut, since the JSON error
envelope needs unsent headers.

The `/payments` collection mixes operation types that name the same concepts
differently, so participant and amount columns are normalised per type: a
`create_account` reports `funder`/`account`/`starting_balance` and an
`account_merge` reports `account`/`into`.

### `GET /admin/export`
Streams transaction records from the database as a CSV or NDJSON download for external accounting.
- **Query Parameters:**
  - `format` (optional) – `csv` (default) or `json`.
  - `limit` (optional) – positive integer row limit. When omitted, only the server cap applies.
  - `startDate` (optional) – `YYYY-MM-DD` inclusive lower bound on `createdAt`.
  - `endDate` (optional) – `YYYY-MM-DD` inclusive upper bound on `createdAt`.
- **Headers:** `x-api-key` (required) – must match `ADMIN_API_KEY`.
- **Returns:** `text/csv` or `application/x-ndjson` with a `Content-Disposition: attachment` header.
- **Status Codes:**
  - `200 OK`: Stream started.
  - `400 Bad Request`: Invalid date format or `startDate` after `endDate`.
  - `401 Unauthorized`: Missing or invalid API key.

Records are fetched at most 500 at a time and written directly to the response. The export stops at the lower of `limit` and `ADMIN_EXPORT_MAX_ROWS` (default `1000000`). Configure the cap as a positive integer no greater than `2147483647`; invalid configuration fails closed with HTTP 500. Invalid limits return HTTP 400. A bounded one-row probe checks for additional matching records before streaming starts, so truncated downloads include `X-Truncated: true`; exports ending exactly at the boundary omit that header. JSON output remains newline-delimited (one object per line).

### `GET /admin/stats/routing`
Returns historical payment routing statistics and aggregated volumes, fees, and transaction counts grouped by day, week, or month.
- **Query Parameters:**
  - `startDate` (optional) – `YYYY-MM-DD` inclusive lower bound on `createdAt`.
  - `endDate` (optional) – `YYYY-MM-DD` inclusive upper bound on `createdAt`.
  - `groupBy` (optional) – `'day'` (default), `'week'`, or `'month'`.
  - `interval` (optional) – Alias for `groupBy`.
  - `weekStart` (optional) – Only meaningful with `groupBy=week`. `'monday'` (default) or `'sunday'`.
  - `assetCode` (optional) – Filter transactions by asset code (e.g., `XLM`, `USDC`).
- **Headers:** `x-api-key` (required) – must match `ADMIN_API_KEY` (or pass `api_key` in query params).
- **Returns:** JSON object containing `interval`, `startDate`, `endDate`, `summary` (`total_volume`, `total_fees`, `total_count`), and `data` array of periodic records (`[{ period, volume, fees, count }]`).
- **Status Codes:**
  - `200 OK`: Statistics retrieved successfully.
  - `400 Bad Request`: Invalid date format, `startDate` after `endDate`, invalid `groupBy`, or invalid `weekStart`.
  - `401 Unauthorized`: Missing or invalid API key.

**Week boundaries:** with `groupBy=week`, periods are ISO weeks starting on
Monday by default. Pass `weekStart=sunday` to group into Sunday-based weeks
instead. Both conventions are computed in SQL; a date that falls in different
weeks under each convention (e.g. a Sunday) will appear in different periods.

### `GET /admin/audit-logs`
Retrieves recent immutable audit trail records for mutating admin actions (`POST`, `PUT`, `DELETE`, `PATCH`).
- **Query Parameters:**
  - `limit` (optional) – Maximum number of records to return (1-100, default 50).
- **Headers:** `x-api-key` (required) – must match `ADMIN_API_KEY` (or pass `api_key` in query params).
- **Returns:** JSON object with `success: true`, `count`, and `data` array of audit records containing `action`, `method`, `path`, `userId`, `ipAddress`, `userAgent`, `statusCode`, `payload` (sensitive data redacted), and `createdAt`.
- **Status Codes:**
  - `200 OK`: Audit logs retrieved successfully.
  - `401 Unauthorized`: Missing or invalid API key.

Mutating admin requests are intercepted by `auditLogMiddleware` and recorded asynchronously upon response completion. Sensitive keys (`password`, `secret`, `apiKey`, `token`, `signature`, `privateKey`, `seed`) are deeply redacted before persistence.

### `GET /metrics`

Prometheus scrape endpoint, served in the Prometheus text format. Exempt from the
rate limiter so a scraper on a fixed interval is never throttled.
- **Returns:** all metrics below, prefixed `stellar_tags_`.
- **Status Codes:** `200 OK`.

| Metric | Type | Description |
| --- | --- | --- |
| `process_resident_memory_bytes`, `nodejs_heap_size_used_bytes`, ... | gauge | Memory usage |
| `process_cpu_user_seconds_total`, `process_cpu_system_seconds_total` | counter | CPU usage |
| `http_requests_total` | counter | Requests by `method`, `route`, `status_code` |
| `http_request_duration_seconds` | histogram | Request latency by `method`, `route`, `status_code`; buckets at 10ms, 50ms, 100ms, 500ms, 1s, 5s |
| `db_pool_connections_open` | gauge | Connections open in the Prisma pool |
| `db_pool_connections_busy` | gauge | Connections executing a query |
| `db_pool_connections_idle` | gauge | Connections open but unused |
| `db_pool_queries_waiting` | gauge | Queries queued waiting for a connection |
| `redis_connections_active` | gauge | `1` while Redis is ready for commands, else `0` |

Memory and CPU come from `prom-client`'s default collectors. The pool gauges read
Prisma's `$metrics` (which requires the `metrics` preview feature in
`schema.prisma`) and report `0` when it is unavailable.

### Monitoring and alerts

The repository ships Prometheus alerting rules in
[docs/prometheus-alerts.yml](docs/prometheus-alerts.yml). Load them with your
scrape config:

```yaml
rule_files:
  - /etc/prometheus/docs/prometheus-alerts.yml
```

| Alert | Expression | For | Meaning |
| --- | --- | --- | --- |
| `StellarTagsDBPoolWaiting` | `stellar_tags_db_pool_queries_waiting > 5` | `30s` | Queries are queuing for a free DB connection; the pool is saturated and query timeouts are imminent. |

The alert is annotated with a summary and runbook hint, and carries a
`severity: warning` label so it can be routed by severity. To create the same
alert in Grafana instead, follow
[docs/grafana-alerts.md](docs/grafana-alerts.md), which walks through importing
the rule and wiring it to a notification channel. The alert panel is also
embedded in [docs/grafana-dashboard.json](docs/grafana-dashboard.json).

### Deprecated endpoints

Deprecated endpoints (configured in `stellar-payment-platform/src/config/deprecations.js`) respond with RFC 8594 `Deprecation`, `Sunset` (RFC 1123 HTTP-date) and `Link` headers.

| Method | Path | Sunset date | Replacement |
|---|---|---|---|
| GET | `/api/v1/lookup` | 2027-02-28 | `/api/v2/lookup` |
| GET | `/api/v1/stats` | 2027-01-31 | `/api/v2/stats` |
| POST | `/api/v1/payments/bulk` | 2027-03-31 | `/api/v2/payments/bulk` |

## Smart Contract Refund Mechanism

When a recipient cannot receive routed tokens (e.g. missing trustline, invalid contract recipient, or transfer rejection), the `PaymentRouter` smart contract prevents whole-transaction aborts by capturing the unrouteable tokens into the contract and crediting the sender's internal refund ledger (`DataKey::RefundBalance(user, token)`).

### Claiming Refunds
Users can query and withdraw their credited refunds at any time using the pull-based withdrawal pattern:
- `get_refund_balance(user: Address, token: Address) -> i128`: Query available internal refund balance (requires `user` authentication to prevent unauthorized surveillance).
- `withdraw_refund(user: Address, token: Address, amount: i128) -> Result<(), Error>`: Withdraw a specific amount of credited tokens.
- `claim_all_refunds(user: Address, token: Address) -> Result<i128, Error>`: Claim and withdraw the entire available refund balance in a single transaction.

## Smart Contract Deployment & Upgrades

The repository includes a dedicated CLI tool (`scripts/deploy.js` and `./scripts/deploy_contract.sh`) to automate WASM compilation, optimization, network deployment, contract initialization, and contract upgrades.

### CLI Usage

```bash
# Display help and available options
./scripts/deploy_contract.sh --help

# Deploy contract to testnet (compiles, optimizes, deploys, and updates .env configs)
./scripts/deploy_contract.sh deploy --network testnet

# Dry-run deployment (simulates workflow without on-chain transactions)
./scripts/deploy_contract.sh deploy --network testnet --dry-run

# Deploy with custom admin and funding source
./scripts/deploy_contract.sh deploy --network testnet --source S... --admin G... --treasury G...

# Deploy to mainnet
./scripts/deploy_contract.sh deploy --network mainnet --source S... --admin G...

# Upgrade an existing contract to newly compiled WASM
./scripts/deploy_contract.sh upgrade --contract-id C... --network testnet --source S...

# Compile and optimize WASM only
./scripts/deploy_contract.sh build
```

### Automation & Config Updates

Upon successful deployment, the tool automatically updates the contract address across:
- `stellar-payment-platform/.env` (`PAYMENT_ROUTER_CONTRACT_ID`, `CONTRACT_ID`)
- `payment-dashboard/.env` (`VITE_CONTRACT_ID`, `CONTRACT_ID`)
- `payment-dashboard/src/views/shared.js` (`CONTRACT_ID`)

## Architecture notes

- The React dashboard runs on `http://localhost:3000` in dev (Vite) and provides the UI.
- The dashboard calls the Node.js API at `http://localhost:5000` via `VITE_API_BASE` and a `/api` proxy.
- The Soroban contract handles on-chain payment routing logic.

## License

See [LICENSE](LICENSE).

## Handsoff notes

<!-- handsoff-issue-110 -->
- #110: webhookWorker.js does not emit notification when webhook is moved to DLQ

<!-- handsoff-issue-64 -->
- #64: registrationService.js does not trim whitespace from usernames before storage

<!-- handsoff-issue-66 -->
- #66: Missing OPTIONS preflight CORS response on admin endpoints returns 405

<!-- handsoff-issue-67 -->
- #67: Soroban contract does not enforce a minimum payment amount (allows amount=0)

<!-- handsoff-issue-229 -->
- #229: [#62] `RegistrationPage.jsx` does not debounce the username availability check

<!-- handsoff-issue-250 -->
- #250: [#83] Smart contract `route_batch` does not return per-recipient result status
