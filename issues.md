# Stellar Tags — GitHub Issues

---

## Issue #1: `GET /federation` returns 500 instead of 404 when username format is valid but unregistered

**Summary:**
The `/federation` endpoint currently falls through to a 500 Internal Server Error when a correctly formatted username tag is queried but does not exist in the database. This is misleading to API consumers who would expect a 404 Not Found. A 500 masks the real problem and breaks client retry logic. The correct behavior is a clear 404 with an actionable message.

**Scope of Work:**
- Trace the query path in `server.js` for `GET /federation`
- Ensure a missing row returns `ApiError('NOT_FOUND', ...)` instead of throwing
- Add or update the zod schema to validate the `q` param format (`name*domain`)
- Update the test in `federation.test.js` to assert 404 on unknown tag

**Out of Scope:**
- Changes to the federation protocol format
- Any changes to the `/register` or `/lookup` endpoints

**Acceptance Criteria:**
- [ ] `GET /federation?q=unknown*localhost` returns `404` with `{ "error": { "code": "NOT_FOUND" } }`
- [ ] `GET /federation?q=` returns `400` with `INVALID_INPUT`
- [ ] Existing passing federation tests still pass
- [ ] No 500 is logged for a missing-tag query

---

## Issue #2: `POST /register` does not enforce Stellar address checksum validation

**Summary:**
The registration endpoint accepts any string as `address` without verifying it is a valid Stellar public key using `StrKey`. An invalid address can be stored in the database and later cause failures at payment time. This is a data-integrity gap that should be caught at entry, not downstream. The fix belongs in the route handler before the Prisma insert.

**Scope of Work:**
- Import `StrKey` from `@stellar/stellar-sdk` in the register handler
- Call `StrKey.isValidEd25519PublicKey(address)` before any DB operation
- Return `ApiError('INVALID_INPUT', 'Invalid Stellar address')` on failure
- Add a test case in `register.test.js` for an invalid address string

**Out of Scope:**
- Validating memo fields in this issue
- Retroactively cleaning up existing invalid rows

**Acceptance Criteria:**
- [ ] `POST /register` with `address: "BADKEY"` returns `400 INVALID_INPUT`
- [ ] A valid `G...` key still registers successfully
- [ ] Error message is user-friendly and references the field name
- [ ] Unit test covers both valid and invalid address inputs

---

## Issue #3: Rate limiter does not differentiate between authenticated and unauthenticated requests

**Summary:**
The current rate limiter applies the same window and limit to all callers regardless of authentication status. Authenticated users with API keys should receive a higher quota than anonymous callers to avoid penalizing legitimate high-volume integrations. This asymmetry is standard practice and prevents good actors from being throttled alongside bad ones. The fix requires reading the `X-Api-Key` header in the rate-limit middleware.

**Scope of Work:**
- Update the rate-limit middleware in `server.js` or dedicated middleware file
- Define separate `windowMs` / `max` values for authenticated vs. unauthenticated
- Use the hashed API key as part of the rate-limit key for authenticated requests
- Add tests in `rate-limit.test.js` covering both tiers

**Out of Scope:**
- Per-endpoint rate limiting (separate issue)
- Changing the API key validation logic itself

**Acceptance Criteria:**
- [ ] Unauthenticated requests hit the lower limit first
- [ ] Authenticated requests with a valid `X-Api-Key` share the higher quota
- [ ] `429` responses include `Retry-After` header
- [ ] Existing rate-limit tests still pass

---

## Issue #4: Webhook delivery does not retry on transient 5xx from the merchant endpoint

**Summary:**
The webhook worker in `src/webhookWorker.js` marks a delivery as failed on the first non-2xx response, regardless of whether it is a transient server error. Merchant endpoints can briefly return 503 during deploys or cold starts, and the system should retry with exponential backoff before moving the event to the dead-letter queue. Without retries, merchants miss events they should receive.

**Scope of Work:**
- Add retry logic (up to 5 attempts) with exponential backoff in `webhookWorker.js`
- Retry on 429, 500, 502, 503, 504 from the merchant endpoint
- Record attempt count and last error in the `WebhookEvent` row
- Move to DLQ only after all retries exhausted
- Update `webhook-worker.test.js` and `webhook-dlq.test.js`

**Out of Scope:**
- Changing the HMAC signature logic
- Adding new webhook event types

**Acceptance Criteria:**
- [ ] A 503 response triggers a retry after the configured backoff
- [ ] After 5 failed attempts the event appears in the DLQ table
- [ ] Successful delivery on retry 3 marks the event as delivered
- [ ] `attempt_count` column is incremented correctly on each retry

---

## Issue #5: `GET /admin/audit-logs` exposes raw IP addresses without any masking

**Summary:**
The audit log endpoint returns the full `ipAddress` field of every admin action, which may include internal service IPs or contributor home IPs logged during development. Depending on jurisdiction, storing and surfacing raw IPs may have GDPR/CCPA implications. The field should be masked to the last octet (e.g., `192.168.1.xxx`) before being sent in the response. The stored value in the database is not changed.

**Scope of Work:**
- Add an `maskIp(ip)` utility in `src/utils.js`
- Apply the mask in the audit-log route handler before JSON serialization
- Update `audit-log.test.js` to assert the masked format in responses
- Document the masking behavior in `docs/`

**Out of Scope:**
- Changing what is stored in the `AuditLog` table
- Masking IPs in the application logs

**Acceptance Criteria:**
- [ ] Response `ipAddress` field ends with `.xxx` for IPv4
- [ ] IPv6 addresses have the last group masked
- [ ] Unit test for `maskIp` covers IPv4, IPv6, and `null` input
- [ ] No raw IPs appear in the API response

---

## Issue #6: `GET /transactions/export` does not stream; entire result is buffered in memory

**Summary:**
Despite the README stating heap plateaus around 18 MB, the current implementation collects all Horizon pages into an array before writing to the response. For accounts with tens of thousands of transactions this causes memory spikes and request timeouts. The export must pipe data through a transform stream, flushing each page as it arrives without accumulating the full result set.

**Scope of Work:**
- Refactor the export handler to use Node.js `Transform` stream or async generator
- Flush CSV rows per Horizon page, not after all pages are fetched
- Respect socket backpressure using `stream.write()` return value
- Add a test in `transactions-export.test.js` that mocks 3 Horizon pages

**Out of Scope:**
- Changing the CSV column schema
- Adding authentication to this endpoint

**Acceptance Criteria:**
- [ ] First CSV chunk arrives before all Horizon pages are fetched
- [ ] Memory usage stays bounded under a 1,000-row mock export
- [ ] `EXPORT_MAX_PAGES` still truncates and logs a warning
- [ ] Test verifies chunked delivery order

---

## Issue #7: Database connection pool exhaustion is not surfaced in `/health` response

**Summary:**
The `/health` endpoint checks whether Postgres is reachable with a `SELECT 1` but does not inspect pool saturation. A fully saturated pool will pass the health check while requests queue indefinitely, creating a false healthy signal. Pool metrics are already collected in `src/db-pool-monitor.js`; they need to feed into the health response. This prevents silent degradation from going undetected by uptime monitors.

**Scope of Work:**
- Read `db_pool_queries_waiting` from the pool monitor in the health handler
- Add a `pool` field to the health response: `{ status, waitingQueries, busyConnections }`
- Return `503` if `waitingQueries` exceeds a configurable threshold (`POOL_WARN_THRESHOLD`)
- Update `health.test.js` to cover the saturated-pool scenario

**Out of Scope:**
- Changes to the Prometheus metrics endpoint
- Altering the existing `database` field behavior

**Acceptance Criteria:**
- [ ] `GET /health` includes a `pool` key in the response body
- [ ] Response is `503` when waiting queries exceed the threshold
- [ ] `POOL_WARN_THRESHOLD=0` can be set to always surface pool data
- [ ] Test mocks a saturated pool and asserts 503

---

## Issue #8: `POST /register` allows reserved system usernames (e.g. `admin`, `root`, `support`)

**Summary:**
There is no blocklist preventing registration of usernames that could be confused with platform or system accounts. A malicious actor could register `admin*stellartags.com` and use it to phish users. The zod schema for registration should reject a configurable list of reserved names with a `FORBIDDEN` error. This is a security hardening measure with low implementation cost.

**Scope of Work:**
- Define `RESERVED_USERNAMES` array in `src/schemas/index.js`
- Add a `.refine()` check to the `registerBodySchema`
- Return `ApiError('FORBIDDEN', 'This username is reserved.')` on match
- Add test cases in `register.test.js` for each reserved name category

**Out of Scope:**
- User-facing UI changes
- Retroactively unregistering existing reserved names

**Acceptance Criteria:**
- [ ] `POST /register` with `username: "admin"` returns `403 FORBIDDEN`
- [ ] `username: "root"`, `"support"`, `"api"` are also blocked
- [ ] A non-reserved username registers successfully
- [ ] The blocklist is documented in the README

---

## Issue #9: Missing `Content-Security-Policy` header on all API responses

**Summary:**
The Helmet middleware is configured in `src/middleware/security.js` but `Content-Security-Policy` is either absent or set to the permissive default. While the API does not serve HTML, a CSP header prevents certain classes of browser-based attacks on any documentation pages or Swagger UI that may be served from the same origin. Adding a strict policy is a low-effort security improvement. The helmet test in `tests/helmet.test.js` should be extended to assert the policy.

**Scope of Work:**
- Configure `helmet.contentSecurityPolicy()` with a restrictive policy in `security.js`
- Ensure `default-src 'none'` is set for the API-only routes
- Update `tests/helmet.test.js` to assert the `Content-Security-Policy` header
- Document the policy in `docs/`

**Out of Scope:**
- Adding a Swagger or OpenAPI UI
- Changes to CORS configuration

**Acceptance Criteria:**
- [ ] All responses include `Content-Security-Policy` header
- [ ] Header value contains `default-src 'none'`
- [ ] Helmet test passes with the new assertion
- [ ] No existing functionality breaks due to the new header

---

## Issue #10: `GET /lookup` reverse-federation does not return alias usernames

**Summary:**
When an address has multiple registered usernames (aliases), `GET /lookup` only returns the primary username. The API documentation states the primary is returned for reverse lookups, but there is no way for a caller to discover the full list of aliases for an address. A new optional query parameter `?all=true` should return all usernames associated with the address.

**Scope of Work:**
- Add `?all=true` query param to the `/lookup` handler
- When `all=true`, query for all non-deleted usernames for the address
- Return `{ address, usernames: [...], primary: "..." }` shape
- Update the zod query schema and add tests in `register.test.js` or a new file

**Out of Scope:**
- Changing the default single-username behavior
- Pagination for the alias list (max 5 per address anyway)

**Acceptance Criteria:**
- [ ] `GET /lookup?address=G...&all=true` returns all aliases
- [ ] `GET /lookup?address=G...` still returns just the primary
- [ ] Soft-deleted usernames are excluded from the list
- [ ] Test covers address with 3 aliases

---

## Issue #11: Soroban contract `route_payment` does not validate recipient trustline before transfer

**Summary:**
The `PaymentRouter` contract in `payment_router/src/lib.rs` attempts a token transfer to the recipient without first checking whether the recipient has established a trustline for non-native assets. A missing trustline causes the entire transaction to fail rather than triggering the refund mechanism. The contract should check the trustline and route to the refund ledger instead of aborting the transaction.

**Scope of Work:**
- Add a trustline check helper in `lib.rs` before each transfer
- On missing trustline, call the refund-credit path instead of aborting
- Write a unit test simulating a recipient without a trustline
- Update `test_snapshots/` with the new expected ledger state

**Out of Scope:**
- Changes to the fee calculation logic
- Modifying the `withdraw_refund` interface

**Acceptance Criteria:**
- [ ] Payment to recipient without trustline credits the sender's refund ledger
- [ ] Transaction does not abort; it succeeds with a refund event
- [ ] `cargo test` passes with the new trustline test
- [ ] Test snapshot reflects the refund ledger update

---

## Issue #12: `MIGRATION_POLICY=strict` does not exit non-zero before port binding

**Summary:**
When `MIGRATION_POLICY=strict` is set and the database has pending migrations, the server should exit before binding a port. Currently, the migration check in `src/migrate-check.js` logs an error but the process continues, binds the port, and then fails on the first query. This defeats the purpose of the `strict` mode and causes confusing startup logs. The fix is to call `process.exit(1)` before `app.listen()`.

**Scope of Work:**
- Review `src/migrate-check.js` and `server.js` startup sequence
- Ensure `strict` mode calls `process.exit(1)` before `app.listen()`
- Add a test in `migrate-check.test.js` for the strict-exit path
- Update the README to clarify the behavior

**Out of Scope:**
- Changes to the `warn` or `off` policy behaviors
- Altering the migration runner itself

**Acceptance Criteria:**
- [ ] Server with `MIGRATION_POLICY=strict` and pending migrations exits with code 1
- [ ] Exit happens before any port is bound
- [ ] Log message names the pending migration files
- [ ] Test asserts `process.exit` was called with code 1

---

## Issue #13: Webhook HMAC signature uses SHA-256 but does not include a timestamp to prevent replay attacks

**Summary:**
The webhook signature in `X-Webhook-Signature` is computed over the raw payload body but does not include a delivery timestamp. Without a timestamp component, a captured valid request can be replayed indefinitely against a merchant's endpoint. Industry standard (e.g., Stripe) includes a `t=<unix_ts>` component in the signed string and the header. Merchants should reject deliveries older than a configurable tolerance window.

**Scope of Work:**
- Add `X-Webhook-Timestamp` header to every delivery in `webhookWorker.js`
- Include the timestamp in the HMAC input: `${timestamp}.${rawBody}`
- Update `webhook-signature.test.js` to assert the new header and signed format
- Update `docs/webhook-signature-verification.md` with the new verification steps

**Out of Scope:**
- Changing the secret rotation mechanism
- Modifying the backward-compatible `X-Stellar-Tags-Signature` alias

**Acceptance Criteria:**
- [ ] Every delivery includes `X-Webhook-Timestamp` with a Unix timestamp
- [ ] Signature is computed over `${timestamp}.${body}`
- [ ] Verification doc shows how to reject stale deliveries (>5 min tolerance)
- [ ] Existing signature tests updated and passing

---

## Issue #14: `GET /admin/stats/routing` returns incorrect weekly grouping near month boundaries

**Summary:**
The `groupBy=week` aggregation in the routing stats endpoint produces inaccurate counts when a week spans two calendar months. The SQL date truncation used in `src/services/statsService.js` does not align with ISO week boundaries, causing records from the last days of a month to be bucketed into the wrong week. This produces incorrect totals in the Grafana dashboard. The fix requires switching to ISO week truncation in the query.

**Scope of Work:**
- Review the `statsService.js` aggregation query for the `week` group-by
- Switch to `DATE_TRUNC('week', ...)` with ISO week semantics in PostgreSQL
- Add a test in `admin-stats-routing.test.js` using a date range that spans a month boundary
- Verify the Grafana dashboard JSON still maps correctly

**Out of Scope:**
- Changes to the `day` or `month` groupings
- Frontend chart rendering

**Acceptance Criteria:**
- [ ] Weekly totals are correct across a January/February boundary test case
- [ ] `groupBy=week` response periods start on Monday (ISO)
- [ ] Existing daily and monthly grouping tests still pass
- [ ] No regression in stats cache behavior

---

## Issue #15: Frontend `App.jsx` does not handle wallet connection timeout gracefully

**Summary:**
When Freighter or another Stellar wallet extension takes longer than expected to respond, the dashboard in `App.jsx` hangs indefinitely with no feedback to the user. There is no timeout or error boundary around the wallet connection call. This results in a frozen UI that users cannot recover from without refreshing the page. A timeout with a user-visible error message should be implemented.

**Scope of Work:**
- Wrap the wallet connect call in `App.jsx` with a `Promise.race()` timeout (10s)
- Display an inline error message when the timeout fires
- Add a loading spinner during the connection attempt
- Write a Vitest/RTL unit test mocking a slow wallet response

**Out of Scope:**
- Supporting additional wallet adapters
- Changes to the backend API

**Acceptance Criteria:**
- [ ] UI shows a spinner while connecting
- [ ] After 10 s with no response, an error message is displayed
- [ ] User can retry the connection without refreshing
- [ ] Test asserts error message appears after mocked timeout

---

## Issue #16: `GET /users/:username/activity` leaks IP addresses of other users in admin context

**Summary:**
The activity endpoint is intended to return only the authenticated user's own activity trail, but under certain query conditions the pagination logic in `src/pagination.js` can return rows belonging to other usernames when the database cursor is not scoped to the requesting user. This is a data-isolation bug. Every query to the activity table must include a `WHERE username = :username` clause enforced at the service layer.

**Scope of Work:**
- Audit `src/services/activityService.js` query to confirm `username` scoping
- Add an explicit filter on `username` that cannot be overridden by pagination params
- Write a test that creates two users and asserts user A cannot see user B's rows
- Add an integration test in `activity-endpoint.test.js`

**Out of Scope:**
- Admin-level activity aggregation (separate issue)
- Changing the pagination utility itself

**Acceptance Criteria:**
- [ ] Activity query always includes `WHERE username = ?` from the service layer
- [ ] User A with valid signature cannot retrieve User B's activity rows
- [ ] Test asserts cross-user isolation
- [ ] Existing activity tests still pass

---

## Issue #17: Docker `dev` profile does not pin PostgreSQL version, causing environment drift

**Summary:**
The `docker-compose.yml` `dev` profile specifies `postgres:16` without a minor version tag, meaning different contributors pull different patch releases. Schema behavior and extension availability can differ across minor versions, leading to "works on my machine" bugs. All Postgres images in the Compose file should be pinned to an exact digest or at minimum a `16.x` tag.

**Scope of Work:**
- Update `docker-compose.yml` to pin `postgres:16.3` (or latest stable 16.x)
- Do the same for the `test` profile database service
- Document the pinned version in the README under "Database setup"
- Add a CI step or comment to remind contributors to update the pin on upgrades

**Out of Scope:**
- Upgrading to Postgres 17
- Changes to the Prisma schema

**Acceptance Criteria:**
- [ ] `docker-compose.yml` references `postgres:16.3` (exact minor version)
- [ ] Both `dev` and `test` profiles use the same pinned tag
- [ ] `docker compose --profile dev up` starts successfully with the pinned image
- [ ] README documents the pinned version

---

## Issue #18: `src/logger.js` does not redact sensitive fields before writing to log files

**Summary:**
The Winston logger writes request bodies and metadata to `logs/application-*.log` without stripping sensitive fields such as `password`, `privateKey`, `seed`, and `signature`. If a developer accidentally logs a full request object, credentials end up on disk in plain text. A redaction transform should be added to the logger to scrub known sensitive keys recursively before serialization.

**Scope of Work:**
- Add a `redact` option or custom format to the Winston logger in `src/logger.js`
- Recursively scrub keys matching the same list used in `auditLog.js`
- Write a unit test asserting that a log call with a `password` field produces `[REDACTED]`
- Ensure the redaction applies to both file and console transports

**Out of Scope:**
- Changing the log rotation configuration
- Redacting HTTP response bodies

**Acceptance Criteria:**
- [ ] `logger.info('test', { password: 'secret' })` writes `password: "[REDACTED]"`
- [ ] Nested objects are also redacted
- [ ] Console transport also redacts sensitive fields
- [ ] Unit test for the redaction transform passes

---

## Issue #19: CI `backend-tests.yml` workflow does not run with a real Postgres container

**Summary:**
The backend test workflow in `.github/workflows/backend-tests.yml` runs Jest against mocked database calls, meaning Prisma query logic is never exercised against a real PostgreSQL instance in CI. Bugs in migration SQL or Prisma query syntax only surface in production or manual testing. A `services:` block with a Postgres container should be added to the workflow so integration tests run against a real database.

**Scope of Work:**
- Add a `services: postgres:` block to `backend-tests.yml`
- Set `DATABASE_URL` env var pointing to the service container
- Run `npm run prisma:deploy` before the test step
- Ensure the test profile database does not conflict with the CI service

**Out of Scope:**
- Adding Redis to CI (separate issue)
- Changing the test framework

**Acceptance Criteria:**
- [ ] Workflow starts a `postgres:16.3` service container
- [ ] `DATABASE_URL` is set correctly in the test environment
- [ ] `npm run prisma:deploy` runs before `npm test`
- [ ] All existing tests pass in the updated CI workflow

---

## Issue #20: `packages/types` TypeScript bindings are not validated against the compiled WASM in CI

**Summary:**
The README states that the `bindings-check` CI job fails the build when checked-in bindings drift from the contract ABI, but no such job exists in `.github/workflows/`. The `packages/types/src/index.ts` bindings can silently drift from `payment_router/src/lib.rs` after a contract change, breaking the frontend at runtime. The `bindings-check` job must be created to enforce freshness.

**Scope of Work:**
- Create `.github/workflows/bindings-check.yml` that builds the contract WASM
- Run `npm run generate:bindings` and check for a git diff in `packages/types`
- Fail the job if `git diff --exit-code packages/types` is non-zero
- Document the workflow in the README

**Out of Scope:**
- Changes to the bindings generation script itself
- Publishing the package to npm

**Acceptance Criteria:**
- [ ] New workflow file exists at `.github/workflows/bindings-check.yml`
- [ ] Job fails when `packages/types` has uncommitted changes after regeneration
- [ ] Job passes when bindings are up to date
- [ ] README references the new CI job

---

## Issue #21: `GET /health` Horizon probe uses a hard-coded URL instead of `HORIZON_BASE` env var

**Summary:**
The Horizon health check in the health endpoint makes an HTTP request to a URL that is hard-coded in the handler rather than reading `HORIZON_BASE` from the environment. This means staging and production environments that point to different Horizon instances always probe the wrong endpoint. The probe URL must be derived from `process.env.HORIZON_BASE` with a sensible default.

**Scope of Work:**
- Replace the hard-coded Horizon URL with `process.env.HORIZON_BASE`
- Default to `https://horizon-testnet.stellar.org` if unset
- Update `health.test.js` to mock the env var and verify the probe URL
- Add `HORIZON_BASE` to `.env.example` with a comment

**Out of Scope:**
- Changing the `HEALTH_HORIZON_TIMEOUT_MS` behavior
- Adding other health probe targets

**Acceptance Criteria:**
- [ ] Health check probes `${HORIZON_BASE}/` when the env var is set
- [ ] Falls back to testnet URL when `HORIZON_BASE` is unset
- [ ] Test mocks `HORIZON_BASE=https://custom.horizon` and asserts the correct URL
- [ ] `.env.example` includes the new variable

---

## Issue #22: Seed script can run against production databases without explicit opt-in

**Summary:**
`scripts/seed.js` checks for a non-local `DATABASE_URL` and refuses to run unless `SEED_ALLOW_REMOTE=1` is set, but the check is a simple string match against `localhost` that can be bypassed by aliasing localhost in DNS or `/etc/hosts`. A more robust guard should hash the connection string or check the Prisma environment label. Additionally, the script should print a loud warning before any destructive operation.

**Scope of Work:**
- Replace the `localhost` string check with a configurable `SEED_ALLOWED_HOSTS` allowlist
- Print a `⚠ WARNING: This will modify the database` prompt requiring `SEED_ALLOW_REMOTE=1`
- Add a `--dry-run` flag that prints what would be upserted without writing
- Update `seed.test.js` to cover the new guard logic

**Out of Scope:**
- Changing the seed data itself
- Modifying the `--reset` flag behavior

**Acceptance Criteria:**
- [ ] Running seed against a remote URL without `SEED_ALLOW_REMOTE=1` exits with a clear error
- [ ] `--dry-run` flag prints upsert targets and exits cleanly
- [ ] Warning message is printed before any write
- [ ] Unit test mocks `DATABASE_URL` with a remote host and asserts refusal

---

## Issue #23: `POST /register` race condition allows duplicate usernames under concurrent requests

**Summary:**
When two requests to register the same username arrive simultaneously, both can pass the existence check before either commit, resulting in a duplicate username violation at the database layer that surfaces as an unhandled Prisma error rather than a clean `409 CONFLICT`. The fix is to rely on the unique constraint and catch `P2002` Prisma error codes, translating them to the correct API error.

**Scope of Work:**
- Wrap the Prisma `create` in a try/catch for `P2002` (unique constraint violation)
- Map `P2002` to `ApiError('CONFLICT', 'Username is already taken.')`
- Remove the pre-check SELECT that creates the race window
- Add a concurrent-request test simulating two simultaneous registrations

**Out of Scope:**
- Changes to the 5-username-per-address limit logic
- Altering the database schema

**Acceptance Criteria:**
- [ ] Two simultaneous requests for the same username result in one `200` and one `409`
- [ ] No unhandled Prisma error appears in the logs
- [ ] The `409` response uses `CONFLICT` error code
- [ ] Test uses `Promise.all` to simulate concurrency

---

## Issue #24: Frontend `HistoryPage.jsx` fetches all transaction history on mount with no pagination

**Summary:**
`HistoryPage.jsx` calls the transactions export endpoint and attempts to render the full result set at once. For accounts with hundreds of transactions, this causes a long load time, a large DOM, and browser memory pressure. The page must implement cursor-based or page-based pagination matching the `page`/`limit` query parameters supported by the backend.

**Scope of Work:**
- Add `page` and `limit` state variables to `HistoryPage.jsx`
- Fetch one page at a time and render a "Load more" or paginator control
- Display a loading state between page fetches
- Add a Vitest test for the pagination state transitions

**Out of Scope:**
- Backend changes to the export endpoint
- Infinite scroll (pagination buttons are sufficient)

**Acceptance Criteria:**
- [ ] Initial render fetches only the first page (default 20 items)
- [ ] "Next page" button fetches the next batch
- [ ] Loading spinner displays between fetches
- [ ] Test verifies state transitions on page change

---

## Issue #25: `src/webhookWorker.js` does not cap concurrency, allowing unbounded parallel deliveries

**Summary:**
The webhook worker processes all pending events in parallel without any concurrency limit. Under high load, this can create hundreds of simultaneous outbound HTTP connections, exhausting the Node.js event loop and potentially rate-limiting the platform's outbound IP at the merchant side. A configurable concurrency cap (default 10) should throttle simultaneous deliveries using a semaphore or `p-limit`.

**Scope of Work:**
- Install `p-limit` (exact version pinned) or implement a simple semaphore
- Apply the concurrency cap to the delivery loop in `webhookWorker.js`
- Add `WEBHOOK_CONCURRENCY` env var (default `10`) to configure the cap
- Add tests in `webhook-worker.test.js` asserting no more than N simultaneous requests

**Out of Scope:**
- Changing retry logic or DLQ behavior
- Modifying the webhook event schema

**Acceptance Criteria:**
- [ ] No more than `WEBHOOK_CONCURRENCY` deliveries run simultaneously
- [ ] Default concurrency is 10 when the env var is unset
- [ ] Test uses a mock to count concurrent in-flight requests
- [ ] `.env.example` documents the new variable

---

## Issue #26: `GET /admin/export` does not support NDJSON streaming for large datasets

**Summary:**
The admin export endpoint declares `format=json` support in the README but the actual implementation in `src/utils/exporter.js` only streams CSV. When `format=json` is requested, the exporter either throws or falls back to CSV silently. Merchants using the NDJSON format for accounting pipelines receive incorrect data. The JSON streaming path must be implemented with proper `application/x-ndjson` content type.

**Scope of Work:**
- Implement the NDJSON branch in `src/utils/exporter.js`
- Set `Content-Type: application/x-ndjson` and the correct `Content-Disposition`
- Write one JSON object per line followed by `\n`
- Add tests in `admin-export.test.js` for the `format=json` path

**Out of Scope:**
- Adding new export formats (XML, Parquet, etc.)
- Changing the date filter logic

**Acceptance Criteria:**
- [ ] `GET /admin/export?format=json` streams valid NDJSON
- [ ] Each line is a complete, parseable JSON object
- [ ] `Content-Type` header is `application/x-ndjson`
- [ ] Test verifies line-by-line JSON validity

---

## Issue #27: Prisma schema missing index on `WebhookEvent.status` causing slow DLQ queries

**Summary:**
The `WebhookEvent` table is queried by `status` (e.g., `pending`, `failed`) on every worker poll cycle. Without an index on `status`, these queries perform full table scans that will degrade as event volume grows. The Prisma schema in `prisma/schema.prisma` should add a `@@index([status])` directive and a migration should be generated.

**Scope of Work:**
- Add `@@index([status])` to the `WebhookEvent` model in `schema.prisma`
- Run `npm run prisma:migrate` to generate the migration SQL
- Verify the index is present in the migration file
- Add a comment in `schema.prisma` explaining the query pattern

**Out of Scope:**
- Adding composite indexes
- Query plan analysis beyond confirming index existence

**Acceptance Criteria:**
- [ ] `schema.prisma` has `@@index([status])` on `WebhookEvent`
- [ ] A new migration file exists with `CREATE INDEX` SQL
- [ ] `npm run prisma:migrate` applies cleanly on a fresh database
- [ ] No existing tests broken by the migration

---

## Issue #28: `GET /metrics` leaks internal route names in `route` label

**Summary:**
The Prometheus `http_requests_total` metric uses the raw Express route path as the `route` label, including dynamic segments like `/users/:username/activity`. High-cardinality dynamic values (actual usernames) can accidentally be included if the router is not configured correctly, causing label explosion that degrades Prometheus performance. Route labels must be normalized to their pattern strings.

**Scope of Work:**
- Audit `src/metrics.js` to confirm labels use route pattern, not actual path
- Ensure `req.route.path` (not `req.path`) is used for the label value
- Add a fallback label `unknown` for requests that don't match a route
- Update `metrics.test.js` to assert normalized route labels

**Out of Scope:**
- Adding new metrics
- Changes to the Grafana dashboard JSON

**Acceptance Criteria:**
- [ ] `GET /users/alice/activity` records label `route="/users/:username/activity"`
- [ ] No actual username values appear in metric labels
- [ ] Unmatched routes use `unknown` label
- [ ] Test asserts the normalized label value

---

## Issue #29: Contract deployment script does not verify WASM hash after upload

**Summary:**
`scripts/deploy.js` uploads the compiled WASM to the Stellar network but does not verify that the on-chain WASM hash matches the local file's SHA-256. A corrupted upload or man-in-the-middle could result in a different contract being initialized. The deploy script should compute the expected hash locally and compare it to the hash returned by the network after upload.

**Scope of Work:**
- Compute SHA-256 of the local `.wasm` file before upload in `deploy.js`
- After upload, retrieve the on-chain hash via the Stellar RPC
- Compare and abort with a clear error if they differ
- Add a `--skip-hash-check` flag for emergency bypasses

**Out of Scope:**
- Changing the contract initialization logic
- Modifying the upgrade path

**Acceptance Criteria:**
- [ ] Deploy script prints the expected and actual WASM hashes
- [ ] Script exits non-zero if hashes differ
- [ ] `--skip-hash-check` flag bypasses the check with a warning
- [ ] Unit test mocks the RPC response and verifies hash comparison

---

## Issue #30: `src/middleware/errorHandler.js` does not log correlation IDs for 4xx errors

**Summary:**
The error handler only attaches `reference_id` (and logs the full stack) for 5xx errors. For 4xx errors, the `correlation_id` is included in the response but not written to the application log. This makes it impossible to trace a specific 400 or 404 through the logs using the correlation ID from a client bug report. All errors should log their correlation ID at the appropriate level.

**Scope of Work:**
- Update `src/middleware/errorHandler.js` to log `correlation_id` for all errors
- Use `warn` level for 4xx and `error` for 5xx
- Include `{ correlationId, statusCode, code }` in the log metadata
- Update `errorHandler.test.js` to assert the log call for a 4xx

**Out of Scope:**
- Changing the response body format
- Adding new error codes

**Acceptance Criteria:**
- [ ] A `404` response is logged at `warn` with its `correlationId`
- [ ] A `500` response is logged at `error` with its `correlationId` and stack
- [ ] Log metadata includes `statusCode` and `code` fields
- [ ] Test spies on `logger.warn` and asserts the correlation ID

---

## Issue #31: Frontend `RegistrationPage.jsx` submits form with empty username on fast double-click

**Summary:**
The registration form in `RegistrationPage.jsx` does not disable the submit button after the first click, allowing users on slow connections to submit the form multiple times. This can result in multiple API calls to `POST /register`, potentially hitting the 5-alias limit unintentionally or creating race conditions. The button should be disabled immediately on first submission and re-enabled only on error.

**Scope of Work:**
- Add a `submitting` state boolean to `RegistrationPage.jsx`
- Set `submitting=true` on form submit and `false` on response/error
- Disable the submit button when `submitting` is true
- Add an accessibility `aria-busy` attribute during submission

**Out of Scope:**
- Server-side idempotency for registration (separate issue)
- Changing the form validation logic

**Acceptance Criteria:**
- [ ] Submit button is disabled after first click
- [ ] Button re-enables after a failed API call
- [ ] Button stays disabled during loading
- [ ] `aria-busy="true"` is set on the button during submission

---

## Issue #32: `GET /federation` does not set `Cache-Control` headers, causing aggressive CDN caching

**Summary:**
The federation endpoint responds without any `Cache-Control` header. CDN proxies (Cloudflare, Fastly) may cache the `200` response indefinitely with their default TTL, returning stale username-to-address mappings after a user transfers their username. The response must include `Cache-Control: no-store` or a short `max-age` to prevent stale reads.

**Scope of Work:**
- Add `Cache-Control: public, max-age=60, stale-while-revalidate=30` to `GET /federation`
- Add `Cache-Control: no-store` to `POST /register` and mutation endpoints
- Update tests in `federation.test.js` to assert the header is present
- Document caching behavior in the endpoint reference

**Out of Scope:**
- Server-side caching changes (Redis federation cache is separate)
- Changes to other endpoints

**Acceptance Criteria:**
- [ ] `GET /federation` response includes `Cache-Control: public, max-age=60`
- [ ] `POST /register` response includes `Cache-Control: no-store`
- [ ] Tests assert the header value
- [ ] No test regressions

---

## Issue #33: `payment_router` fuzz target does not cover the batch payment path

**Summary:**
The existing fuzz target in `payment_router/fuzz/fuzz_targets/route_payments.rs` only exercises single-recipient payments. The `route_batch` function, which processes multiple recipients in one transaction, is untested by the fuzzer. Batch payments involve more complex state transitions and are a higher-risk surface for arithmetic overflows or panics.

**Scope of Work:**
- Add a `route_batch` fuzz target in `fuzz/fuzz_targets/`
- Generate arbitrary batch sizes (1–20 recipients) with random amounts
- Include edge cases: all-zero amounts, max `i128` values, duplicate recipients
- Update `.github/workflows/fuzz.yml` to run the new target

**Out of Scope:**
- Fuzzing the admin or emergency-withdraw functions
- Changes to the fuzz corpus

**Acceptance Criteria:**
- [ ] New fuzz target file exists at `fuzz/fuzz_targets/route_batch.rs`
- [ ] Fuzz target compiles with `cargo fuzz build`
- [ ] CI workflow runs the new target for 30 seconds
- [ ] Any panic found is reproduced and documented as a separate bug

---

## Issue #34: `src/services/stellarService.js` does not handle Horizon 429 rate limits

**Summary:**
When Horizon returns a `429 Too Many Requests` response, the current `stellarService.js` propagates it as an `UPSTREAM_ERROR`, causing the API to return `502` to clients. Instead, the service should implement backoff-retry for Horizon 429s (up to 3 retries with exponential delay) before escalating. This prevents transient Horizon throttling from surfacing as permanent API failures.

**Scope of Work:**
- Add a retry wrapper around Horizon calls in `stellarService.js`
- Retry on 429 with `Retry-After` header respect or exponential backoff
- After 3 retries, return `ApiError('SERVICE_UNAVAILABLE', 'Horizon rate limit exceeded')`
- Update `stellarService.test.js` to cover the retry path

**Out of Scope:**
- Implementing a local Horizon rate-limit budget
- Caching Horizon responses

**Acceptance Criteria:**
- [ ] A single 429 from Horizon triggers a retry
- [ ] After 3 consecutive 429s, the API returns `503`
- [ ] Backoff delays are observable in the test (mock timers)
- [ ] Test covers the success-on-retry-2 path

---

## Issue #35: `docker-compose.yml` `test` profile exposes database port 5433 without authentication

**Summary:**
The `test` profile in `docker-compose.yml` maps the test database to host port `5433` with the default `POSTGRES_PASSWORD=postgres`. Any process on the host can connect to the test database without credentials, which is a security risk on shared CI machines or developer laptops with network exposure. The port binding should be restricted to `127.0.0.1:5433` or removed entirely.

**Scope of Work:**
- Change `"5433:5432"` to `"127.0.0.1:5433:5432"` in `docker-compose.yml`
- Do the same for the `dev` profile on port `5432`
- Verify the test suite still connects correctly after the change
- Document the reasoning in a compose comment

**Out of Scope:**
- Changing database credentials
- Modifying the application code

**Acceptance Criteria:**
- [ ] `docker-compose.yml` binds Postgres ports to `127.0.0.1` only
- [ ] `docker compose --profile test up` + `npm test` still passes
- [ ] External network scan does not show port 5433 open
- [ ] Comment in compose file explains the restriction

---

## Issue #36: Missing `X-Request-ID` propagation from frontend to backend for tracing

**Summary:**
The React dashboard makes API calls to the backend but does not attach any request correlation header. When debugging a user-reported issue, it is impossible to correlate a specific frontend action with a backend log entry. The frontend should generate a `uuid` per request and attach it as `X-Request-ID`, and the backend should echo it in responses and logs alongside `correlationId`.

**Scope of Work:**
- Add a request interceptor in `payment-dashboard/src/` to attach `X-Request-ID: uuid()`
- Update the backend `correlation.js` middleware to read `X-Request-ID` if present
- Echo `X-Request-ID` in response headers
- Add a test asserting the header round-trip

**Out of Scope:**
- Distributed tracing (OpenTelemetry) setup
- Changes to the Prometheus metrics

**Acceptance Criteria:**
- [ ] Frontend attaches `X-Request-ID` to every API call
- [ ] Backend logs include the `X-Request-ID` value
- [ ] Response headers include `X-Request-ID`
- [ ] Test asserts header presence in both directions

---

## Issue #37: Soft-deleted users can still be looked up via `GET /federation`

**Summary:**
When a user is soft-deleted (the `deletedAt` field is set), their record remains in the database and is still resolved by `GET /federation`. A deleted user's tag should not resolve to an address; callers should receive a `404`. The query in the federation handler must filter on `deletedAt IS NULL`.

**Scope of Work:**
- Add `where: { deletedAt: null }` to the federation lookup query
- Verify the same filter exists in `GET /lookup` and `POST /register` alias checks
- Add a test in `soft-delete-lookup.test.js` for the federation path
- Confirm soft-delete purge cron does not interfere with the fix

**Out of Scope:**
- Hard-delete behavior
- Changes to the purge cron schedule

**Acceptance Criteria:**
- [ ] `GET /federation?q=deleted_user*domain` returns `404`
- [ ] Undeleted users still resolve correctly
- [ ] Test creates a soft-deleted user and asserts 404
- [ ] No regressions in federation compliance tests

---

## Issue #38: `GET /admin/stats/routing` does not validate `assetCode` input, allowing SQL injection vector

**Summary:**
The `assetCode` query parameter in the routing stats endpoint is passed directly into a raw SQL fragment without parameterization in `statsService.js`. While Prisma's query builder is used elsewhere, this filter uses string interpolation. An attacker with a valid admin API key could craft a malicious `assetCode` to exfiltrate data. The parameter must be validated and parameterized.

**Scope of Work:**
- Add `assetCode` to the zod schema for the routing stats endpoint
- Enforce a regex allowlist: uppercase letters and digits, 1–12 chars
- Replace string interpolation with a Prisma parameterized where clause
- Add a test in `sql-injection.test.js` for this endpoint

**Out of Scope:**
- Changing other query parameters
- Auditing other endpoints for injection (separate issue)

**Acceptance Criteria:**
- [ ] `assetCode` with SQL metacharacters returns `400 INVALID_INPUT`
- [ ] Valid `assetCode=USDC` still filters correctly
- [ ] SQL injection test passes
- [ ] Prisma query uses parameterized binding

---

## Issue #39: `npm run generate:bindings` script fails silently when `stellar` CLI is not on PATH

**Summary:**
`scripts/generate-bindings.sh` calls the `stellar` CLI without checking if it is installed. When the CLI is missing, the script exits with a non-zero code but no human-readable error message, leaving the developer without actionable guidance. The script should check for the CLI up front and print installation instructions on failure.

**Scope of Work:**
- Add a `command -v stellar` check at the top of `generate-bindings.sh`
- Print a clear install message (`cargo install stellar-cli --version X.Y.Z`) if missing
- Exit with code `1` and a non-zero status
- Add the same check to `deploy_contract.sh`

**Out of Scope:**
- Automating the CLI installation
- Changing the bindings generation logic

**Acceptance Criteria:**
- [ ] Script prints "stellar CLI not found. Install with: cargo install ..." when missing
- [ ] Exit code is `1` when the CLI is absent
- [ ] When CLI is present, existing behavior is unchanged
- [ ] `deploy_contract.sh` has the same guard

---

## Issue #40: `src/middleware/deprecation.js` does not include sunset date in `Deprecation` header

**Summary:**
The deprecation middleware sets a `Deprecation: true` header on deprecated routes but does not include a `Sunset` header with the planned removal date, as required by RFC 8594. Consumers cannot programmatically determine when a deprecated endpoint will be removed. The middleware should read the sunset date from `src/config/deprecations.js` and set the `Sunset` header.

**Scope of Work:**
- Update `src/config/deprecations.js` to include `sunsetDate` per deprecated route
- Update `deprecation.js` middleware to set `Sunset: <HTTP-date>` header
- Update `deprecation.test.js` to assert the `Sunset` header
- Document the sunset dates in the API reference

**Out of Scope:**
- Removing deprecated routes (done at the sunset date)
- Changes to the `Deprecation` header format

**Acceptance Criteria:**
- [ ] Deprecated routes return `Sunset: <date>` header
- [ ] Date format is RFC 1123 HTTP-date
- [ ] Test asserts the `Sunset` header matches the configured date
- [ ] `deprecations.js` has a `sunsetDate` field for each deprecated entry

---

## Issue #41: `AnalyticsPage.jsx` chart renders with empty state after API error instead of error message

**Summary:**
When the analytics endpoint returns an error, `AnalyticsPage.jsx` silently renders empty charts because the error is caught and ignored. Users see blank charts with no indication that data failed to load. The component must handle API errors and display a user-friendly error state with a retry button.

**Scope of Work:**
- Add error state handling to `AnalyticsPage.jsx` fetch calls
- Display an error banner with the error message and a "Retry" button
- Implement the retry button to re-trigger the fetch
- Add a unit test mocking a failed API call

**Out of Scope:**
- Changes to the analytics backend endpoint
- Chart library upgrades

**Acceptance Criteria:**
- [ ] An API error shows an error message instead of blank charts
- [ ] "Retry" button triggers a fresh API call
- [ ] Error state clears when the retry succeeds
- [ ] Test asserts error banner appears on fetch failure

---

## Issue #42: `prisma/schema.prisma` `User` model lacks an index on `address` for reverse lookups

**Summary:**
The `/lookup` endpoint queries users by `address` (for reverse federation), but the `User` model in `schema.prisma` does not have an index on the `address` field. As the user count grows, this query will perform a full table scan. An index is needed for acceptable performance at scale.

**Scope of Work:**
- Add `@@index([address])` to the `User` model in `schema.prisma`
- Generate a migration with `npm run prisma:migrate`
- Verify the migration SQL contains `CREATE INDEX`
- Update the performance note in `README.md`

**Out of Scope:**
- Adding composite indexes
- Changing the lookup query logic

**Acceptance Criteria:**
- [ ] `schema.prisma` has `@@index([address])` on `User`
- [ ] Migration file created with correct `CREATE INDEX` SQL
- [ ] `npm run prisma:deploy` applies the migration cleanly
- [ ] Existing lookup tests still pass

---

## Issue #43: `GET /users/:username/activity` missing pagination `totalPages` when result is empty

**Summary:**
When the activity trail for a user is empty, the response returns `meta: { total: 0, page: 1, limit: 10 }` but omits the `totalPages` field. Client code that accesses `meta.totalPages` will receive `undefined` and may throw. The response contract must always include all `meta` fields, defaulting `totalPages` to `0` when there are no results.

**Scope of Work:**
- Trace the `meta` construction in `src/pagination.js`
- Ensure `totalPages: Math.ceil(total / limit)` or `0` is always set
- Add a test in `pagination.test.js` for the zero-results case
- Confirm all paginated endpoints use the same helper

**Out of Scope:**
- Changing the pagination algorithm
- Adding new meta fields

**Acceptance Criteria:**
- [ ] Empty activity response includes `meta.totalPages: 0`
- [ ] `totalPages` is always a number, never undefined
- [ ] Pagination test covers the empty-result case
- [ ] All paginated endpoints share the same meta-builder

---

## Issue #44: Fuzz target `route_payments.rs` does not bound the `amount` input

**Summary:**
The fuzz target in `payment_router/fuzz/fuzz_targets/route_payments.rs` generates arbitrary `i128` amounts without lower-bound filtering. Negative amounts reach the contract and cause a panic in the subtraction path, producing false-positive crash reports. The fuzz target should either filter negative values or expect the contract to return an error for them.

**Scope of Work:**
- Add a check in the fuzz target to skip or clamp negative amounts
- Alternatively, assert the contract returns `Err(Error::InvalidAmount)` for negatives
- Update the fuzz target comment to document the intent
- Regenerate or update the fuzz corpus if needed

**Out of Scope:**
- Changing the contract's amount validation
- Adding new fuzz dimensions

**Acceptance Criteria:**
- [ ] Fuzz target does not report crashes for negative-amount inputs
- [ ] If contract returns an error, the fuzz target treats it as expected
- [ ] `cargo fuzz run route_payments -- -max_total_time=60` completes without false crashes
- [ ] Fuzz target file includes a comment on amount handling

---

## Issue #45: `.github/workflows/ci.yml` does not cache `node_modules` between runs

**Summary:**
The main CI workflow installs Node.js dependencies from scratch on every run, adding 60–90 seconds to each build. Using the standard `actions/cache` action keyed on `package-lock.json` hash would restore the cache on non-dependency-changing commits and significantly speed up the CI feedback loop.

**Scope of Work:**
- Add `actions/cache` step to `ci.yml` for `node_modules` and `~/.npm`
- Use `hashFiles('**/package-lock.json')` as the cache key
- Do the same for `stellar-payment-platform/` and `payment-dashboard/` separately
- Verify the cache is restored on a second run (check CI logs)

**Out of Scope:**
- Caching Cargo dependencies (separate issue)
- Changes to test steps

**Acceptance Criteria:**
- [ ] CI workflow includes `actions/cache` for npm
- [ ] Second run with no dependency changes restores from cache
- [ ] Cache key includes `package-lock.json` hash
- [ ] No test failures due to stale cache

---

## Issue #46: `src/services/activityService.js` does not log activity for username transfer events

**Summary:**
When a username is transferred from one address to another, the system updates the `User` record but does not write a `user.transferred` activity log entry. This creates a gap in the audit trail that is inconsistent with the documented `Actions` list. Transfer events must be recorded in the `ActivityLog` table with the old and new addresses in `metadata`.

**Scope of Work:**
- Identify the transfer handler in `server.js` or the registration service
- Call `activityService.log({ action: 'user.transferred', metadata: { from, to } })` after transfer
- Add a test in `activity.test.js` for the transfer event
- Confirm the event appears in `GET /users/:username/activity`

**Out of Scope:**
- Changing the transfer endpoint behavior
- Adding new activity action types

**Acceptance Criteria:**
- [ ] A username transfer writes a `user.transferred` row to `ActivityLog`
- [ ] `metadata` contains `{ from: "G...", to: "G..." }`
- [ ] The event appears in the activity trail response
- [ ] Test verifies the activity log entry after a transfer

---

## Issue #47: `GET /health` Redis probe does not return `not configured` when `REDIS_URL` is unset on all branches

**Summary:**
The health endpoint should return `redis: "not configured"` when `REDIS_URL` is absent. Testing shows that some code paths return `redis: "down"` instead when the env var is unset, because the Redis client attempts to connect to `localhost:6379` by default. The health probe must distinguish between "Redis is configured but unreachable" and "Redis is not configured at all".

**Scope of Work:**
- Check for `REDIS_URL` presence before creating the Redis client in `src/config/redis.js`
- Return `null` or a sentinel value when unconfigured
- Update the health handler to map `null` → `"not configured"`
- Update `health.test.js` to assert `"not configured"` when `REDIS_URL` is unset

**Out of Scope:**
- Adding Redis as a required dependency
- Changing the Redis connection pooling

**Acceptance Criteria:**
- [ ] `GET /health` returns `{ redis: "not configured" }` when `REDIS_URL` is unset
- [ ] `{ redis: "down" }` is returned only when `REDIS_URL` is set but Redis is unreachable
- [ ] Test covers the unconfigured and down scenarios
- [ ] No false `503` when Redis is intentionally not used

---

## Issue #48: `render.yaml` does not set `MIGRATION_POLICY=strict` for production deploys

**Summary:**
The Render deployment blueprint in `render.yaml` does not set `MIGRATION_POLICY`. This means production deployments default to `warn` mode, where a schema drift is logged but the server starts anyway. A production deploy with pending migrations should fail fast rather than serving requests against a mismatched schema. Setting `MIGRATION_POLICY=strict` in `render.yaml` enforces this.

**Scope of Work:**
- Add `MIGRATION_POLICY: strict` to the env section of `render.yaml`
- Verify the startup script in `startup.sh` respects the env var
- Document the behavior in `DEPLOYMENT_CHECKLIST.md`
- Add a note in README under "Render deployment"

**Out of Scope:**
- Changing the migration check logic
- Modifying the `startup.sh` migration runner

**Acceptance Criteria:**
- [ ] `render.yaml` includes `MIGRATION_POLICY: strict`
- [ ] `DEPLOYMENT_CHECKLIST.md` references strict mode
- [ ] README Render section mentions the policy
- [ ] No change to local dev defaults

---

## Issue #49: `src/utils/jwt.js` uses `HS256` algorithm without documenting key size requirements

**Summary:**
The JWT utility in `src/utils/jwt.js` signs tokens with `HS256` but there is no validation of the `JWT_SECRET` length. A short secret (under 32 bytes) makes the HMAC weak and guessable. The utility should enforce a minimum key length at startup and document the requirement in `.env.example`.

**Scope of Work:**
- Add a startup check in `jwt.js` that throws if `JWT_SECRET.length < 32`
- Update `.env.example` to document the minimum key length
- Add a test in `jwt-utils.test.js` for the short-key rejection
- Log a startup warning if the secret is between 32 and 64 bytes

**Out of Scope:**
- Migrating to RS256/ES256 (separate security issue)
- Changing the token expiry logic

**Acceptance Criteria:**
- [ ] Server throws on startup if `JWT_SECRET` is under 32 bytes
- [ ] `.env.example` documents `# Must be at least 32 characters`
- [ ] Test verifies the startup error with a short secret
- [ ] Warning logged for 32–64 byte secrets

---

## Issue #50: Multi-signer verification does not check the transaction sequence number

**Summary:**
The `src/multisigner-verifier.js` validates multi-sig threshold and key weights but does not verify that the signed transaction's sequence number matches the account's current sequence on Horizon. Replaying an old signed transaction with a lower sequence number can pass verification even though it would be rejected on-chain. The sequence number must be fetched and compared during verification.

**Scope of Work:**
- Fetch the account's current sequence from Horizon in `multisigner-verifier.js`
- Compare the transaction's `sequence` field to `account.sequence + 1`
- Return a verification failure if they do not match
- Add tests in `multisigner-verifier.test.js` for stale-sequence scenarios

**Out of Scope:**
- Changes to the multi-signer enrollment flow
- Modifying the weight threshold calculation

**Acceptance Criteria:**
- [ ] Verification fails for a transaction with a stale sequence number
- [ ] Verification succeeds when sequence is correct
- [ ] Test mocks Horizon account response with a specific sequence
- [ ] Error message clearly states "invalid sequence number"

---

## Issue #51: `GET /federation` does not support `type=id` reverse lookup per federation protocol

**Summary:**
The Stellar federation protocol requires servers to handle `?type=id` queries that resolve an account ID to a federation address. The current `/federation` endpoint only handles `type=name` lookups and silently ignores the `type` parameter. This breaks third-party wallet compatibility with the Stellar federation spec (SEP-2). Support for `type=id` must be added.

**Scope of Work:**
- Read `type` query parameter in the federation handler
- When `type=id`, delegate to the reverse-lookup (`/lookup`) logic
- Return the primary username in the `stellar_address` field
- Add tests in `federation-compliance.test.js` for the `type=id` path

**Out of Scope:**
- Supporting `type=txid` (not required by SEP-2 for this use case)
- Changes to the `/lookup` endpoint

**Acceptance Criteria:**
- [ ] `GET /federation?q=G...&type=id` returns the registered federation address
- [ ] `type=name` (default) continues to work as before
- [ ] Unknown `type` value returns `400 INVALID_INPUT`
- [ ] Federation compliance test covers `type=id`

---

## Issue #52: `src/cleanup-cron.js` does not log which records were purged

**Summary:**
The cleanup cron in `src/cleanup-cron.js` runs soft-delete purges but only logs that the cron ran, not how many records were deleted or which tables were affected. Without per-table counts in the log, it is impossible to detect runaway purges or verify that cleanup is working. The cron should log `{ table, count }` for each delete operation.

**Scope of Work:**
- Capture the `count` from each Prisma `deleteMany` result in `cleanup-cron.js`
- Log `logger.info('Purge completed', { table: 'User', count: N })` for each table
- Add a test in `cleanup-cron.test.js` that asserts the log output
- Add a summary log at cron completion: total records purged

**Out of Scope:**
- Changing the retention window
- Adding new tables to the purge

**Acceptance Criteria:**
- [ ] Log contains per-table purge counts after each run
- [ ] Total count is logged at completion
- [ ] Test spies on `logger.info` and asserts count fields
- [ ] No change to existing purge behavior

---

## Issue #53: `payment-dashboard` build does not fail on TypeScript errors in `.jsx` files

**Summary:**
The Vite config in `payment-dashboard/vite.config.js` transpiles JSX but does not run type checking. TypeScript errors in `.jsx` files (which use JSDoc type annotations) are silently ignored during builds, allowing type-incorrect code to reach production. A `tsc --noEmit` step should be added to the build process and CI.

**Scope of Work:**
- Add a `tsconfig.json` to `payment-dashboard/` with `checkJs: true` and `noEmit: true`
- Add `"typecheck": "tsc --noEmit"` to `payment-dashboard/package.json` scripts
- Call `npm run typecheck` in the CI workflow before build
- Fix any existing type errors discovered

**Out of Scope:**
- Converting files from `.jsx` to `.tsx`
- Changing the Vite build configuration

**Acceptance Criteria:**
- [ ] `npm run typecheck` in `payment-dashboard/` runs without error
- [ ] A deliberate type error causes the CI step to fail
- [ ] CI workflow calls `typecheck` before the build step
- [ ] No existing functionality changed

---

## Issue #54: `X-Api-Key` header value is logged in plain text on authentication failure

**Summary:**
When an invalid `X-Api-Key` is provided, the authentication middleware logs the rejection with the raw key value included in the log metadata. Even a partial key logged in plain text is a security risk, especially if logs are shipped to a third-party log aggregator. The key must be truncated to the first 8 characters followed by `...` before being logged.

**Scope of Work:**
- Find all log calls in the API key authentication code path
- Replace `apiKey` log value with `apiKey.substring(0, 8) + '...'`
- Add a helper `truncateKey(key)` to `src/utils.js`
- Add a test asserting the key is truncated in the log on failure

**Out of Scope:**
- Changing how API keys are stored or validated
- Redacting keys from error response bodies

**Acceptance Criteria:**
- [ ] Failed auth log includes `apiKey: "abcd1234..."` not the full value
- [ ] `truncateKey` utility handles `null`/`undefined` input safely
- [ ] Test asserts the truncated format
- [ ] No plain-text key in any log call

---

## Issue #55: `GET /admin/audit-logs` has no time-range filter, making it useless for incident investigation

**Summary:**
The audit log endpoint only accepts a `limit` parameter, returning the most recent N records. For incident investigation, operators need to filter by time range (e.g., "show all admin actions between 2026-09-01 and 2026-09-03"). Without date filters, investigators must fetch pages manually to find the relevant time window. `startDate` and `endDate` query parameters must be added.

**Scope of Work:**
- Add `startDate` and `endDate` query params to the audit log route
- Apply date filters as `createdAt >= startDate AND createdAt <= endDate` in the Prisma query
- Validate dates with the existing zod date schema pattern
- Add tests in `audit-log.test.js` for date-filtered queries

**Out of Scope:**
- Adding full-text search to audit logs
- Exporting audit logs as CSV

**Acceptance Criteria:**
- [ ] `?startDate=2026-09-01&endDate=2026-09-03` returns only records in that range
- [ ] Missing date params return recent records as before
- [ ] Invalid date format returns `400 INVALID_INPUT`
- [ ] Test covers both bounded and unbounded queries

---

## Issue #56: Contract `claim_all_refunds` does not emit an event for off-chain tracking

**Summary:**
The `claim_all_refunds` function in `payment_router/src/lib.rs` executes the full refund withdrawal but does not emit a Soroban contract event. Off-chain services (like `horizonListener.js`) that track contract state cannot detect refund claims without scanning the ledger manually. A `refund_claimed` event should be emitted with `user`, `token`, and `amount` fields.

**Scope of Work:**
- Add a `events::publish` call in `claim_all_refunds` in `lib.rs`
- Use topic `["refund_claimed", user, token]` and data `amount`
- Write a unit test asserting the event is emitted
- Update `horizonListener.js` to handle the new event type

**Out of Scope:**
- Changes to `withdraw_refund` (similar but separate)
- Frontend dashboard changes

**Acceptance Criteria:**
- [ ] `claim_all_refunds` emits a `refund_claimed` event
- [ ] Event contains correct `user`, `token`, and `amount` fields
- [ ] Unit test asserts event emission in the test environment
- [ ] `horizonListener.js` logs the event when received

---

## Issue #57: `src/schemas/index.js` does not validate `page` and `limit` as positive integers

**Summary:**
The zod schemas for paginated endpoints accept `page` and `limit` as numbers but do not enforce that they are positive integers. A caller sending `?page=-1&limit=0` can trigger unexpected Prisma query behavior (negative skip values). The README states these clamp to bounds, but the clamping is inconsistent across endpoints. The schema should coerce and clamp both values.

**Scope of Work:**
- Update the pagination schema in `src/schemas/index.js` to enforce `min(1)` on `page` and `min(1)` on `limit`
- Cap `limit` at `100` using `.max(100)`
- Use `.default(1)` and `.default(10)` for missing values
- Update `schemas.test.js` to cover boundary inputs

**Out of Scope:**
- Changing pagination logic in individual handlers
- Adding cursor-based pagination

**Acceptance Criteria:**
- [ ] `?page=-1` is clamped to `page=1`
- [ ] `?limit=0` is clamped to `limit=1`
- [ ] `?limit=1000` is clamped to `limit=100`
- [ ] Schema tests assert all boundary cases

---

## Issue #58: `src/middleware/bodyLimit.js` does not reject `Transfer-Encoding: chunked` requests above the cap

**Summary:**
The body size middleware in `src/middleware/bodyLimit.js` enforces a 10 KB cap using `Content-Length`, but chunked transfer encoding does not include `Content-Length`. An attacker can bypass the limit by sending a chunked body over 10 KB. The middleware must buffer and enforce the limit on chunked bodies as well, or reject chunked requests on JSON endpoints.

**Scope of Work:**
- Update `bodyLimit.js` to track bytes received on `data` events for chunked requests
- Abort and return `413 PAYLOAD_TOO_LARGE` when the running total exceeds the cap
- Add a test in `bodyLimit.test.js` simulating a chunked request over the limit
- Verify the fix does not break streaming endpoints (`/transactions/export`)

**Out of Scope:**
- Changing the body size cap value
- Affecting non-JSON endpoints

**Acceptance Criteria:**
- [ ] A chunked request with >10 KB body returns `413`
- [ ] A chunked request under 10 KB is processed normally
- [ ] Streaming export endpoints are exempt from the chunk limit
- [ ] Test simulates chunked delivery and asserts 413

---

## Issue #59: `scripts/deploy_contract.sh` hard-codes network passphrase for testnet

**Summary:**
The deploy script in `scripts/deploy_contract.sh` hard-codes the Stellar testnet network passphrase string rather than reading it from the `stellar` CLI's network configuration. When deploying to mainnet with `--network mainnet`, the passphrase is still the testnet value, causing transaction signing to fail with a cryptic error. The passphrase must be derived from the selected network.

**Scope of Work:**
- Remove the hard-coded passphrase from `deploy_contract.sh`
- Use `stellar network ls` or the network flag to derive the passphrase
- Add a passphrase validation step before the first RPC call
- Test the fix by running `--dry-run --network testnet`

**Out of Scope:**
- Changing the contract build logic
- Modifying `scripts/deploy.js`

**Acceptance Criteria:**
- [ ] `--network testnet` uses the correct testnet passphrase
- [ ] `--network mainnet` uses the mainnet passphrase
- [ ] Script prints the derived passphrase (first 10 chars) in verbose mode
- [ ] `--dry-run` completes without a passphrase error

---

## Issue #60: `payment-dashboard` missing `robots.txt` and `sitemap.xml` for SEO and crawler control

**Summary:**
The Vite app's `public/` directory does not include a `robots.txt` file. Without it, search engine crawlers index all pages including internal routes like wallet connection flows and registration pages. A `robots.txt` should disallow sensitive paths and a `sitemap.xml` should list only the public-facing pages.

**Scope of Work:**
- Add `payment-dashboard/public/robots.txt` disallowing `/register` and `/history`
- Add `payment-dashboard/public/sitemap.xml` listing the landing and help pages
- Update `vite.config.js` to copy the files to the build output
- Verify with `npm run build` that files appear in `dist/`

**Out of Scope:**
- SEO meta tag optimization
- Server-side rendering

**Acceptance Criteria:**
- [ ] `robots.txt` disallows `/register` and `/history`
- [ ] `sitemap.xml` lists at least the root and `/help` URLs
- [ ] Both files are present in `dist/` after build
- [ ] No existing routes are broken

---

## Issue #61: `horizonListener.js` does not persist cursor position, reprocessing events on restart

**Summary:**
The Horizon payment listener in `horizonListener.js` starts streaming from the latest ledger on each startup, meaning any events that occurred while the server was down are never processed. The cursor position (last processed paging token) should be persisted to Redis or the database and resumed on restart to ensure no events are missed.

**Scope of Work:**
- Store the last processed `paging_token` in Redis (key: `horizon:cursor`)
- On startup, read the stored cursor and pass it to the Horizon stream call
- Update the cursor after each successfully processed event
- Add a test mocking Redis read/write for the cursor

**Out of Scope:**
- Reprocessing historical events older than the cursor
- Changing the event processing logic

**Acceptance Criteria:**
- [ ] Last paging token is written to Redis after each event
- [ ] On restart, the listener resumes from the stored cursor
- [ ] If no cursor is stored, starts from `now`
- [ ] Test verifies cursor is read on startup and updated on each event

---

## Issue #62: `GET /federation` does not handle `*` wildcard domain queries gracefully

**Summary:**
Some federation clients send queries with a wildcard domain (e.g., `alice*`) when probing for server capabilities. The current handler attempts a database lookup with an invalid username format and returns a 500. This should be caught by input validation and returned as `400 INVALID_INPUT` with a message about the required `name*domain` format.

**Scope of Work:**
- Update the federation query validation schema to enforce the `*` separator and non-empty domain
- Return `400 INVALID_INPUT` for queries without a valid domain part
- Add tests in `federation.test.js` for malformed `q` values
- Document the required format in the endpoint reference

**Out of Scope:**
- Implementing wildcard domain federation
- Changes to the database query

**Acceptance Criteria:**
- [ ] `GET /federation?q=alice*` returns `400 INVALID_INPUT`
- [ ] `GET /federation?q=*domain` returns `400 INVALID_INPUT`
- [ ] `GET /federation?q=alice*domain` resolves normally
- [ ] Tests cover all three cases

---

## Issue #63: `src/services/registrationService.js` does not trim whitespace from usernames before storage

**Summary:**
If a user submits a username with leading or trailing whitespace (e.g., `" alice "`), it is stored as-is in the database. This creates an invisible duplicate that cannot be looked up without the whitespace and cannot be unregistered normally. All username inputs must be trimmed before validation and storage.

**Scope of Work:**
- Add `.trim()` to the username value in `registrationService.js` before any DB operation
- Ensure the zod schema also includes `.trim()` as a transform
- Add a test with a padded username asserting it is stored without whitespace
- Check the same for `GET /federation` and `GET /lookup`

**Out of Scope:**
- Retroactively fixing existing records with whitespace
- Normalizing case (separate issue)

**Acceptance Criteria:**
- [ ] `POST /register` with `username: " alice "` stores `"alice"`
- [ ] `GET /federation?q= alice *domain` resolves after trim
- [ ] Schema test verifies the trim transform
- [ ] No regressions in existing registration tests

---

## Issue #64: `src/middleware/auditLog.js` records `payload` synchronously, blocking the response

**Summary:**
The audit log middleware in `src/middleware/auditLog.js` writes to the `AuditLog` table synchronously as part of the response lifecycle. A slow database write delays every admin response by the duration of the insert. The audit log write should be fire-and-forget (async, non-blocking) with an error logged if the insert fails, rather than blocking the response.

**Scope of Work:**
- Move the Prisma insert in `auditLog.js` to a `setImmediate` or `process.nextTick` callback
- Catch and log errors without propagating them to the response
- Add a test asserting the response completes before the audit write
- Verify existing audit log tests still pass

**Out of Scope:**
- Changing what is recorded in the audit log
- Adding a queue for audit events

**Acceptance Criteria:**
- [ ] Admin response returns before the audit log insert completes
- [ ] Audit log insert failure is logged but does not affect the response
- [ ] Test uses mock timers to verify async behavior
- [ ] Existing audit log tests pass

---

## Issue #65: Missing `OPTIONS` preflight response for CORS on admin endpoints

**Summary:**
The CORS middleware returns correct headers for actual requests but browser preflight `OPTIONS` requests to `/admin/*` endpoints receive a `405 Method Not Allowed` instead of `200`. This blocks any browser-based admin UI from making credentialed requests. The server must respond to `OPTIONS` with the appropriate CORS headers and `200 OK`.

**Scope of Work:**
- Ensure the CORS middleware runs before the method-not-allowed handler for `OPTIONS`
- Explicitly handle `OPTIONS *` in Express or the CORS config
- Add a test making an OPTIONS preflight to `/admin/audit-logs`
- Verify the `Access-Control-Allow-Methods` header includes `GET`

**Out of Scope:**
- Building a browser-based admin UI
- Changing CORS allowed origins

**Acceptance Criteria:**
- [ ] `OPTIONS /admin/audit-logs` returns `200` with CORS headers
- [ ] `Access-Control-Allow-Methods` includes the endpoint's allowed methods
- [ ] Test asserts `200` status and CORS headers on OPTIONS
- [ ] No regression in existing CORS tests

---

## Issue #66: Soroban contract does not enforce a minimum payment amount

**Summary:**
The `route_payment` function in `lib.rs` allows payments of `0` or very small amounts (1 stroopa). While the Stellar network enforces minimum fees, the contract should also reject zero-amount payments at the contract level to avoid wasting ledger entries on meaningless transactions and to prevent fee-only attacks. A minimum of `1` (1 stroop equivalent) should be enforced.

**Scope of Work:**
- Add an `amount > 0` check at the top of `route_payment` in `lib.rs`
- Return `Err(Error::InvalidAmount)` for non-positive amounts
- Add a unit test for zero and negative amount inputs
- Update the contract ABI documentation

**Out of Scope:**
- Changing the fee calculation
- Setting a maximum payment amount

**Acceptance Criteria:**
- [ ] `route_payment` with `amount=0` returns `Err(Error::InvalidAmount)`
- [ ] `route_payment` with `amount=-1` returns the same error
- [ ] Positive amounts still route correctly
- [ ] Unit test covers all three cases

---

## Issue #67: `GET /transactions/export` does not validate the `order` parameter before passing to Horizon

**Summary:**
The `order` query parameter is passed to the Horizon API call without validation. Only `asc` and `desc` are valid values, but arbitrary strings are forwarded to Horizon, which may either error or return unexpected results. The parameter should be validated with a zod enum schema before the Horizon call.

**Scope of Work:**
- Add `order: z.enum(['asc', 'desc']).default('desc')` to the export query schema
- Return `400 INVALID_INPUT` for invalid `order` values
- Add a test in `transactions-export.test.js` for an invalid order value
- Confirm the existing `asc` and `desc` paths work correctly

**Out of Scope:**
- Adding new sort options
- Changing the CSV column order

**Acceptance Criteria:**
- [ ] `?order=random` returns `400 INVALID_INPUT`
- [ ] `?order=asc` and `?order=desc` work as before
- [ ] Missing `order` defaults to `desc`
- [ ] Test covers invalid, asc, desc, and missing cases

---

## Issue #68: `src/webhookWorker.js` does not validate merchant endpoint URL before delivery

**Summary:**
The webhook worker attempts to deliver to whatever URL is stored in the `Webhook.url` field without validating it at delivery time. If a URL contains a private IP (e.g., `http://192.168.1.1/internal`) or a `file://` scheme, the worker can be exploited as a Server-Side Request Forgery (SSRF) vector. The delivery code must validate the URL is an HTTPS endpoint with a public IP before each delivery.

**Scope of Work:**
- Add a URL validation step in `webhookWorker.js` before the HTTP request
- Reject `http://` (non-TLS), `file://`, and private IP ranges (RFC 1918)
- Log and fail the delivery with a clear error when the URL is rejected
- Add SSRF test cases to `webhook-worker.test.js`

**Out of Scope:**
- Changing the webhook registration endpoint
- Adding URL validation at registration time (separate issue)

**Acceptance Criteria:**
- [ ] Delivery to `http://192.168.1.1/hook` is rejected as SSRF
- [ ] Delivery to `file:///etc/passwd` is rejected
- [ ] Delivery to `https://merchant.example.com/hook` proceeds normally
- [ ] Test covers each rejected URL pattern

---

## Issue #69: CI `soroban.yml` workflow does not run contract unit tests

**Summary:**
The `.github/workflows/soroban.yml` workflow builds the contract WASM and runs `cargo clippy` but does not execute `cargo test`. Contract unit tests in `lib.rs` verify business logic like fee calculation, batch routing, and refund crediting, and must be run in CI to catch regressions after contract changes.

**Scope of Work:**
- Add a `cargo test` step to `soroban.yml` after the build step
- Configure the test step to run with `--features testutils` if needed
- Ensure test snapshots are committed and the step fails on snapshot mismatch
- Run with `-- --nocapture` for better CI log output

**Out of Scope:**
- Adding integration tests to this workflow (those run via the `integration` profile)
- Changing the contract source

**Acceptance Criteria:**
- [ ] `soroban.yml` includes a `cargo test` step
- [ ] Contract unit tests pass in CI
- [ ] Test snapshot mismatch fails the CI step
- [ ] CI logs show individual test names

---

## Issue #70: `src/originCache.js` does not expire cached entries, growing unboundedly

**Summary:**
The in-memory origin cache in `src/originCache.js` stores approved origins without any TTL or maximum size. If thousands of unique origins are approved over time, the cache grows indefinitely and increases memory pressure. A TTL (e.g., 5 minutes) and a max-size eviction policy (LRU, max 1,000 entries) should be added.

**Scope of Work:**
- Add a TTL per entry (5 minutes) using timestamps or a TTL-capable data structure
- Cap the cache at 1,000 entries with LRU eviction
- Add a `ORIGIN_CACHE_TTL_MS` env var to configure TTL
- Add a test in a new `originCache.test.js` for TTL expiry and eviction

**Out of Scope:**
- Moving the cache to Redis
- Changing the CORS origin validation logic

**Acceptance Criteria:**
- [ ] Entries expire after the configured TTL
- [ ] Cache size does not exceed 1,000 entries
- [ ] `ORIGIN_CACHE_TTL_MS` env var is respected
- [ ] Test verifies expiry and eviction behavior

---

## Issue #71: `payment-dashboard` `App.jsx` does not display network badge for mainnet deployments

**Summary:**
The `NetworkBadge.jsx` component exists but is only shown when `VITE_NETWORK=testnet`. On mainnet deployments where `VITE_NETWORK` is unset or set to `mainnet`, the badge is not rendered. Users on mainnet have no visual indication of which network they are connected to, which is a UX and safety gap. The badge should always be shown, styled differently for mainnet (green) vs testnet (yellow).

**Scope of Work:**
- Update `NetworkBadge.jsx` to render for both `testnet` and `mainnet` values
- Style: testnet = yellow/amber badge, mainnet = green badge
- Default to `testnet` if `VITE_NETWORK` is unset
- Add a test asserting the badge renders for both values

**Out of Scope:**
- Supporting additional networks (Futurenet, etc.)
- Changes to the backend network configuration

**Acceptance Criteria:**
- [ ] `VITE_NETWORK=mainnet` renders a green badge
- [ ] `VITE_NETWORK=testnet` renders a yellow badge
- [ ] Unset `VITE_NETWORK` defaults to testnet badge
- [ ] Test covers both badge states

---

## Issue #72: `src/db.js` Prisma client is instantiated multiple times in test environments

**Summary:**
`src/db.js` creates a new `PrismaClient` instance on every `require()` call in test environments because Node's module cache is cleared between test files by Jest's module isolation. This results in multiple open database connections per test run, eventually exhausting the pool. The Prisma client should be a singleton using the global object pattern recommended by Prisma.

**Scope of Work:**
- Update `src/db.js` to use `global.__prisma` singleton pattern
- Prevent multiple client instantiations in the same process
- Verify connection count in `prisma-client.test.js`
- Add a note in the README about the singleton pattern

**Out of Scope:**
- Changing the Prisma schema
- Altering test isolation strategies

**Acceptance Criteria:**
- [ ] `require('./src/db')` returns the same instance on repeated calls
- [ ] Database connection count does not grow linearly with test file count
- [ ] `prisma-client.test.js` asserts singleton behavior
- [ ] No test failures due to connection pool exhaustion

---

## Issue #73: `docs/webhook-signature-verification.md` Go example has incorrect HMAC comparison

**Summary:**
The Go verification example in `docs/webhook-signature-verification.md` uses `==` for string comparison of HMAC values instead of `hmac.Equal()`. A direct string comparison is not constant-time and is vulnerable to timing attacks, allowing an attacker to infer the correct signature byte by byte. The example must be updated to use `hmac.Equal([]byte(expected), []byte(actual))`.

**Scope of Work:**
- Replace the `==` comparison in the Go example with `hmac.Equal`
- Add a comment explaining why constant-time comparison is necessary
- Review the Python example for the same issue (use `hmac.compare_digest`)
- Update `webhook-signature.test.js` if any server-side code has the same bug

**Out of Scope:**
- Rewriting the entire documentation page
- Adding new language examples

**Acceptance Criteria:**
- [ ] Go example uses `hmac.Equal`
- [ ] Python example uses `hmac.compare_digest`
- [ ] Both examples include a comment about timing-safe comparison
- [ ] Server-side HMAC verification code also uses constant-time comparison

---

## Issue #74: `GET /admin/stats/routing` does not cache results, hitting the database on every request

**Summary:**
The routing stats query aggregates over potentially millions of rows and runs on every request with no caching. The `src/cache/statsCache.js` module exists for exactly this purpose but is not wired into the routing stats handler. Repeated requests for the same date range re-run the full aggregation. The stats response should be cached for 5 minutes by default.

**Scope of Work:**
- Wire `statsCache.js` into the routing stats handler in the relevant route file
- Use `startDate + endDate + groupBy + assetCode` as the cache key
- Set a 5-minute TTL (configurable via `STATS_CACHE_TTL_MS`)
- Add tests in `stats-cache.test.js` verifying cache hit/miss behavior

**Out of Scope:**
- Changing the aggregation query
- Adding cache invalidation on data changes

**Acceptance Criteria:**
- [ ] Second identical request is served from cache without a DB query
- [ ] Cache expires after the configured TTL
- [ ] Different query params result in different cache entries
- [ ] Test asserts DB is not queried on cache hit

---

## Issue #75: `POST /register` does not return `is_primary` correctly for alias registrations

**Summary:**
The `POST /register` endpoint documentation states the response includes `is_primary: true` for the first username registered to an address and `is_primary: false` for subsequent aliases. In practice, the field is always `true` regardless of whether the username is an alias, because the primary check logic is inverted. The handler must correctly determine primacy before responding.

**Scope of Work:**
- Review the registration handler's primacy determination logic
- Fix the condition so `is_primary: true` is returned only for the first username per address
- Add a test registering two usernames to the same address and asserting `is_primary` values
- Update `register.test.js` to cover the alias registration path

**Out of Scope:**
- Changes to the primary-username promotion logic
- Changing the response schema

**Acceptance Criteria:**
- [ ] First username for an address returns `is_primary: true`
- [ ] Second username for the same address returns `is_primary: false`
- [ ] `GET /lookup` for the address returns the primary username
- [ ] Test covers both registration calls in sequence

---

## Issue #76: `src/middleware/validateSchema.js` does not strip unknown fields from request body

**Summary:**
The zod schema validator passes the parsed object back to the handler but does not strip unknown fields from the request body. An attacker can inject extra fields (e.g., `isAdmin: true`, `deletedAt: null`) that might be picked up by ORM spread operations. The schema should be configured with `.strict()` or `.strip()` to remove unrecognized keys before the handler runs.

**Scope of Work:**
- Add `.strip()` (or configure `z.object.strict()`) to all body schemas in `src/schemas/index.js`
- Verify that extra fields in the request body are removed before reaching handlers
- Add tests in `validateSchema.test.js` for extra-field stripping
- Audit `server.js` handlers for spread operations that could pick up injected fields

**Out of Scope:**
- Changing query parameter schemas
- Adding new schema fields

**Acceptance Criteria:**
- [ ] Extra fields in a valid request body are stripped, not passed to handlers
- [ ] `POST /register` with `{ username, address, isAdmin: true }` stores only `username` and `address`
- [ ] Test asserts unknown fields are absent in the parsed body
- [ ] No existing handler tests break

---

## Issue #77: `src/federationCache.js` does not invalidate cache on username deregistration

**Summary:**
When a username is deregistered (soft-deleted), the federation response cache in `federationCache.js` continues to serve the stale positive result until the TTL expires. A deregistered tag can be re-registered by another user, at which point callers with a cached response route payments to the wrong address. Cache entries must be invalidated immediately on deregistration.

**Scope of Work:**
- Call `federationCache.invalidate(username)` in the deregistration handler
- Add `invalidate(key)` method to `federationCache.js` if not present
- Add a test in `federation-cache.test.js` asserting cache miss after deregistration
- Verify the same invalidation happens on username transfer

**Out of Scope:**
- Changing the cache TTL
- Invalidating the cache on soft-delete purge (TTL handles that)

**Acceptance Criteria:**
- [ ] Cache is invalidated immediately after deregistration
- [ ] Subsequent federation lookup after deregistration hits the DB, not cache
- [ ] Test creates, caches, deregisters, and re-queries a username
- [ ] Username transfer also invalidates the cache

---

## Issue #78: `src/routes/v2/` directory exists but contains no route files

**Summary:**
The directory `stellar-payment-platform/src/routes/v2/` is empty, but the `mount-versioning.js` file and API versioning middleware suggest v2 routes are planned. The absence of v2 routes while the versioning infrastructure is in place creates confusion for contributors. Either the v2 routes should be scaffolded or the directory should be removed and the versioning middleware simplified until v2 is ready.

**Scope of Work:**
- Decide: scaffold v2 routes or remove the empty directory
- If scaffolding: create at least a `federation.js` v2 route with a deprecation notice on v1
- If removing: update `mount-versioning.js` to only register v1 routes
- Update `api-versioning.test.js` to reflect the decision

**Out of Scope:**
- Implementing v2 endpoint changes
- Changing the deprecation middleware behavior

**Acceptance Criteria:**
- [ ] `src/routes/v2/` either has at least one route file or is removed
- [ ] `mount-versioning.js` correctly mounts only existing routes
- [ ] `api-versioning.test.js` passes with no dead test cases
- [ ] README documents the versioning strategy

---

## Issue #79: `src/utils/exporter.js` does not escape CSV fields containing commas or newlines

**Summary:**
The CSV exporter in `src/utils/exporter.js` writes fields directly without RFC 4180-compliant quoting. If a `metadata` field contains a comma or newline (e.g., a memo with embedded text), the CSV row is malformed and breaks parsers. All fields must be quoted and internal double-quotes must be escaped by doubling (`""`).

**Scope of Work:**
- Add a `csvEscape(field)` utility function that wraps values in quotes and escapes internal quotes
- Apply `csvEscape` to every field in the CSV row writer
- Add a test with a memo containing a comma and a newline
- Verify the exported CSV is parseable by a standard CSV parser

**Out of Scope:**
- Changing the column schema
- Adding NDJSON escaping (separate issue)

**Acceptance Criteria:**
- [ ] A field with a comma is wrapped in double quotes in the output
- [ ] A field with a `"` is escaped as `""`
- [ ] A field with a newline is quoted so the row is not split
- [ ] Test parses the output with a CSV library and verifies field values

---

## Issue #80: `.github/workflows/secret-scan.yml` does not fail CI on new secrets found in PRs

**Summary:**
The `secret-scan.yml` workflow runs TruffleHog but only reports findings as annotations; it does not fail the CI check. A PR containing an accidentally committed API key or private key passes CI silently. The workflow must be configured to exit non-zero when secrets are detected, blocking the merge.

**Scope of Work:**
- Update `secret-scan.yml` to use `--fail` or equivalent flag for TruffleHog
- Ensure the step exits non-zero and marks the CI check as failed
- Test by adding a deliberate (invalidated) test secret to the trufflehog-ignore list
- Document the secret scanning policy in `CONTRIBUTING.md` or README

**Out of Scope:**
- Adding additional secret scanning tools
- Retroactively scanning all commit history

**Acceptance Criteria:**
- [ ] TruffleHog step exits non-zero when secrets are found
- [ ] CI check is marked as failed when the step exits non-zero
- [ ] `trufflehog-ignore.txt` documents the test secret exception
- [ ] README or CONTRIBUTING.md mentions secret scanning

---

## Issue #81: `payment_router` contract missing `get_version` function for on-chain upgrade verification

**Summary:**
After a contract upgrade via `scripts/deploy_contract.sh upgrade`, there is no on-chain way to verify which version of the contract is running. Other contracts and off-chain services (like `contractService.js`) must query the version to ensure they are interacting with the expected API. A `get_version() -> String` function should be added to the contract public interface.

**Scope of Work:**
- Add `pub fn get_version(env: Env) -> String` to `lib.rs`
- Return a semantic version string (e.g., `"1.2.0"`) matching `Cargo.toml`
- Regenerate TypeScript bindings with `npm run generate:bindings`
- Add a unit test asserting the returned version string

**Out of Scope:**
- Implementing upgrade guards based on version
- Changing the contract upgrade logic

**Acceptance Criteria:**
- [ ] `get_version()` returns the contract version from `Cargo.toml`
- [ ] TypeScript bindings include `getVersion()` method
- [ ] Unit test asserts the version string format
- [ ] `cargo test` passes with the new function

---

## Issue #82: `src/services/ownershipService.js` does not handle multi-sig accounts for ownership proof

**Summary:**
The ownership verification in `ownershipService.js` only validates a single Ed25519 signature against the account's master key. Accounts secured by multi-sig thresholds (e.g., 2-of-3 signers) cannot prove ownership via this mechanism, locking multi-sig users out of the activity trail and webhook management. The service should integrate `multisigner-verifier.js` for multi-sig account verification.

**Scope of Work:**
- Detect multi-sig accounts in `ownershipService.js` (account with `signers.length > 1`)
- Delegate to `multisigner-verifier.js` for threshold verification
- Fall back to single-signer verification for standard accounts
- Add tests in `ownership-service.test.js` for multi-sig ownership proof

**Out of Scope:**
- Changes to the multi-signer enrollment flow
- Changing the signature message format

**Acceptance Criteria:**
- [ ] A 2-of-3 multi-sig account can prove ownership with 2 valid signatures
- [ ] A single-signer account continues to use the existing path
- [ ] Test covers both verification paths
- [ ] Ownership service tests pass

---

## Issue #83: `src/middleware/security.js` does not set `Permissions-Policy` header

**Summary:**
The security middleware configures Helmet but does not set a `Permissions-Policy` header. Without this header, browsers may grant the page access to sensitive APIs (camera, microphone, geolocation) that the application never needs. A restrictive `Permissions-Policy` header should be added to prevent feature misuse.

**Scope of Work:**
- Add `Permissions-Policy: camera=(), microphone=(), geolocation=()` to the security middleware
- Configure it via Helmet's custom header support or a separate `res.setHeader` call
- Update `helmet.test.js` to assert the `Permissions-Policy` header
- Review the dashboard for any features that legitimately need browser permissions

**Out of Scope:**
- Changes to the CORS policy
- Dashboard feature changes

**Acceptance Criteria:**
- [ ] All responses include `Permissions-Policy` header
- [ ] Header disables camera, microphone, and geolocation
- [ ] `helmet.test.js` asserts the header value
- [ ] No existing functionality broken

---

## Issue #84: `tests/e2e/` tests do not have a dedicated database teardown step

**Summary:**
The end-to-end tests in `stellar-payment-platform/tests/e2e/` create database records during tests but do not reliably clean them up. Leftover records from a failed test run contaminate subsequent runs, causing flaky test failures. Each e2e test suite must implement `afterAll` teardown that removes records created during the test.

**Scope of Work:**
- Add `afterAll` hooks to each file in `tests/e2e/` that delete created records
- Use unique prefixes (e.g., `e2e_test_` + UUID) for test usernames to scope deletes
- Verify tests pass on consecutive runs without a database reset
- Update the CI workflow to document the e2e test isolation approach

**Out of Scope:**
- Adding database transactions to wrap each test
- Changing e2e test scenarios

**Acceptance Criteria:**
- [ ] Running e2e tests twice in a row produces the same results
- [ ] Test records are deleted in `afterAll` hooks
- [ ] A test failure in `beforeAll` still triggers cleanup
- [ ] No orphaned test records after the test suite completes

---

## Issue #85: `src/multisigner-verifier.js` fetches Horizon account data without a timeout

**Summary:**
The multi-signer verifier makes an HTTP request to Horizon to fetch account signers but does not set a timeout. A hanging Horizon request causes the entire verification to block indefinitely, holding a request slot open and eventually exhausting the Node.js event loop under load. A configurable timeout (default 5 seconds) must be added.

**Scope of Work:**
- Add an `AbortController` with a 5-second timeout to the Horizon fetch in `multisigner-verifier.js`
- Add `MULTISIGNER_HORIZON_TIMEOUT_MS` env var to configure the timeout
- Return a verification error with `UPSTREAM_ERROR` when the timeout fires
- Add a test asserting the timeout behavior

**Out of Scope:**
- Changing the signature verification logic
- Caching Horizon account data

**Acceptance Criteria:**
- [ ] Verification returns an error after 5 seconds with no Horizon response
- [ ] `MULTISIGNER_HORIZON_TIMEOUT_MS` env var configures the timeout
- [ ] Test uses a mock that never resolves and asserts the timeout error
- [ ] Normal fast response still verifies correctly

---

## Issue #86: `GET /metrics` endpoint is accessible without authentication

**Summary:**
The Prometheus `/metrics` endpoint is intentionally exempt from the rate limiter, but it is also accessible without any authentication. Internal metrics may reveal sensitive operational data (active connections, queue depths, route-level traffic patterns) to unauthenticated parties. The endpoint should require a `Bearer` token or be restricted to an internal network range via middleware.

**Scope of Work:**
- Add a `METRICS_TOKEN` env var and require `Authorization: Bearer <token>` on `/metrics`
- If `METRICS_TOKEN` is unset, restrict access to `127.0.0.1` only
- Update `metrics.test.js` to authenticate requests
- Document the metrics security model in `docs/` or README

**Out of Scope:**
- Changing the metrics format or content
- Integrating with Prometheus RBAC

**Acceptance Criteria:**
- [ ] Unauthenticated request to `/metrics` returns `401` when token is set
- [ ] Bearer token authentication grants access
- [ ] When token is unset, only `127.0.0.1` requests succeed
- [ ] Metrics test passes with the updated authentication

---

## Issue #87: `payment-dashboard` `LatencyGauge.jsx` does not debounce frequent rerenders on rapid API calls

**Summary:**
The `LatencyGauge.jsx` component re-renders on every API latency measurement, which `useLatencyTracker.js` updates on every response. On pages with several concurrent API calls, the gauge can flicker and cause layout thrashing. The latency update should be debounced using the existing `useDebounce.js` hook before updating gauge state.

**Scope of Work:**
- Import and apply `useDebounce` to the latency value in `LatencyGauge.jsx`
- Set a 300ms debounce delay to smooth rapid updates
- Add a test asserting the gauge does not rerender more than once within the debounce window
- Verify no visual regression by running the development server

**Out of Scope:**
- Changes to `useLatencyTracker.js` measurement logic
- Changing the gauge visualization

**Acceptance Criteria:**
- [ ] Rapid successive latency updates produce at most one gauge rerender per 300 ms
- [ ] Debounce is applied using the existing `useDebounce` hook
- [ ] Test uses fake timers to assert debounce behavior
- [ ] No visual regression in the gauge display

---

## Issue #88: `src/soft-delete-purge-cron.js` purges records older than 30 days but configurable value is not documented

**Summary:**
The soft-delete purge cron hard-codes a 30-day retention window. Operators on regulated platforms may need to retain records for 90 or 365 days for compliance. The retention period should be configurable via `PURGE_RETENTION_DAYS` env var and the default and valid range should be documented in `.env.example` and the README.

**Scope of Work:**
- Replace the hard-coded `30` in `soft-delete-purge-cron.js` with `process.env.PURGE_RETENTION_DAYS ?? 30`
- Validate the value is a positive integer at startup
- Add `PURGE_RETENTION_DAYS` to `.env.example` with a comment
- Add a test using a custom retention period

**Out of Scope:**
- Changing the purge cron schedule
- Adding per-table retention periods

**Acceptance Criteria:**
- [ ] Setting `PURGE_RETENTION_DAYS=90` uses a 90-day retention window
- [ ] Non-integer or negative values cause a startup error
- [ ] `.env.example` documents the variable with its default
- [ ] Test asserts the configured retention period is used

---

## Issue #89: `src/metrics.js` does not track webhook delivery success/failure rates

**Summary:**
The Prometheus metrics track HTTP request counts and latency but do not include webhook delivery outcomes. Operators cannot alert on high webhook failure rates or detect delivery storms without this data. Three new metrics should be added: `webhook_deliveries_total` (counter, labeled by `status: success|failure`), `webhook_delivery_duration_seconds` (histogram), and `webhook_dlq_size` (gauge).

**Scope of Work:**
- Add the three new metrics to `src/metrics.js`
- Instrument `webhookWorker.js` to increment/observe each metric
- Update the Grafana dashboard JSON to include webhook panels
- Add tests in `metrics.test.js` for the new metrics

**Out of Scope:**
- Changing the Prometheus text format
- Adding per-merchant breakdown labels (cardinality risk)

**Acceptance Criteria:**
- [ ] `webhook_deliveries_total{status="success"}` increments on successful delivery
- [ ] `webhook_deliveries_total{status="failure"}` increments on all-retries-exhausted failure
- [ ] `webhook_dlq_size` reflects the current DLQ depth
- [ ] Metrics test asserts all three new metrics exist

---

## Issue #90: `scripts/deploy.js` does not support a `--dry-run` flag for mainnet safety checks

**Summary:**
The deployment script can be run against mainnet (`--network mainnet`) but there is no `--dry-run` mode that simulates the full workflow without executing on-chain transactions. This makes it risky to test the deployment pipeline against mainnet configuration. A `--dry-run` flag should simulate every step, including WASM compilation and config file updates, but skip RPC calls.

**Scope of Work:**
- Add `--dry-run` flag parsing to `scripts/deploy.js`
- When `--dry-run` is set, skip RPC calls and log `[DRY RUN] Would execute: ...`
- Still compile WASM and validate inputs during dry run
- Add documentation for the flag in the README CLI section

**Out of Scope:**
- Changes to the upgrade path
- Modifying `deploy_contract.sh`

**Acceptance Criteria:**
- [ ] `node scripts/deploy.js --network mainnet --dry-run` completes without RPC calls
- [ ] WASM compilation still runs during dry run
- [ ] All skipped steps are logged with `[DRY RUN]` prefix
- [ ] README documents the `--dry-run` flag

---

## Issue #91: `tests/integration/` Soroban integration tests have no CI trigger

**Summary:**
The integration tests in `tests/integration/` require the `stellar/quickstart` Docker image and test the contract against a local Stellar network. The `soroban.yml` CI workflow does not trigger these tests, and there is no dedicated workflow for them. Regressions in on-chain contract behavior are only caught manually. A workflow using the `integration` Docker Compose profile should be added.

**Scope of Work:**
- Create `.github/workflows/integration-tests.yml`
- Start the `integration` Docker Compose profile in the workflow
- Run `tests/integration/run.sh` and assert exit code 0
- Trigger only on PRs that modify `payment_router/src/`

**Out of Scope:**
- Running integration tests on every commit (too slow)
- Changes to the integration test scripts

**Acceptance Criteria:**
- [ ] New workflow file at `.github/workflows/integration-tests.yml`
- [ ] Workflow triggers on PRs modifying `payment_router/src/**`
- [ ] Integration tests run against local Stellar network
- [ ] Workflow fails if any integration test fails

---

## Issue #92: `src/logger.js` does not include the service name in log records

**Summary:**
Log records written by the Winston logger lack a `service` field. When logs from multiple services (e.g., backend API and a future microservice) are aggregated in a single log platform (Grafana Loki, Datadog), there is no way to filter by service. Adding `service: "stellar-payment-platform"` as a default metadata field solves this.

**Scope of Work:**
- Add `defaultMeta: { service: 'stellar-payment-platform' }` to the Winston logger config
- Add a `version` field sourced from `package.json`
- Update any log format tests to expect the new fields
- Document the logging schema in `docs/`

**Out of Scope:**
- Changing the log rotation configuration
- Adding structured log fields beyond `service` and `version`

**Acceptance Criteria:**
- [ ] Every log record includes `"service": "stellar-payment-platform"`
- [ ] Every log record includes `"version": "<pkg version>"`
- [ ] Existing log format tests pass with the new fields
- [ ] README documents the default log metadata

---

## Issue #93: `src/schemas/index.js` memo validation does not enforce `memo_hash` as 64-character hex

**Summary:**
The registration schema validates `memo_type` against the enum `['text', 'id', 'hash', 'return']` but does not enforce format constraints for each type. A `memo_type: "hash"` should always be paired with a 64-character hexadecimal `memo` value. Storing a non-hex or wrong-length hash causes failures when the memo is used on the Stellar network.

**Scope of Work:**
- Add a zod `.superRefine()` rule to the registration schema checking memo format per type
- `memo_type: "hash"` requires `memo` to match `/^[0-9a-fA-F]{64}$/`
- `memo_type: "id"` requires `memo` to be parseable as a uint64
- Add tests in `validateMemo.test.js` for each type/format combination

**Out of Scope:**
- Changes to the `memo_type: "text"` length limit (separate issue)
- Frontend form validation

**Acceptance Criteria:**
- [ ] `memo_type: "hash"` with a non-hex memo returns `422 VALIDATION_FAILED`
- [ ] `memo_type: "hash"` with a valid 64-char hex memo passes
- [ ] `memo_type: "id"` with a non-numeric memo returns `422`
- [ ] Tests cover all four memo types

---

## Issue #94: Docker `Dockerfile-backend` does not run as a non-root user

**Summary:**
The backend Dockerfile (`Dockerfile-backend`) runs the Node.js server as `root` inside the container. If the application is compromised, an attacker has root access to the container filesystem. The Dockerfile should add a non-root `node` user and switch to it before the `CMD` instruction, following Docker security best practices.

**Scope of Work:**
- Add `RUN addgroup --system node && adduser --system --ingroup node node` to `Dockerfile-backend`
- Add `USER node` before the `CMD` instruction
- Ensure the application files are owned by the `node` user in `COPY` steps
- Verify the container starts and passes health checks with the non-root user

**Out of Scope:**
- Changes to the frontend Dockerfile
- Modifying the application code

**Acceptance Criteria:**
- [ ] Container runs as the `node` user (verifiable with `docker exec whoami`)
- [ ] Application files are readable by the `node` user
- [ ] `GET /health` returns `200` after the change
- [ ] No permission errors in container startup logs

---

## Issue #95: `src/services/stellarService.js` does not handle account-not-found for new addresses

**Summary:**
When `GET /transactions/export` is called for a Stellar address that has never been funded (no ledger entry), Horizon returns a `404`. The current code propagates this as a generic error. The response should be `404 NOT_FOUND` with a clear message: "Account not found on Horizon. The address may not yet be funded." This helps users distinguish between unfunded and invalid addresses.

**Scope of Work:**
- Detect Horizon 404 responses in `stellarService.js`
- Map to `ApiError('NOT_FOUND', 'Account not found on Horizon...')`
- Add a test in `stellarService.test.js` for the Horizon 404 path
- Update the endpoint documentation to describe the 404 case

**Out of Scope:**
- Funding accounts on behalf of users
- Caching Horizon account data

**Acceptance Criteria:**
- [ ] Export for an unfunded address returns `404 NOT_FOUND`
- [ ] Error message mentions the account may not be funded
- [ ] Test mocks a Horizon 404 and asserts the API 404
- [ ] Endpoint docs describe the 404 status code

---

## Issue #96: `payment-dashboard` does not have a production build size analysis step

**Summary:**
The `payment-dashboard/package.json` has no script to analyze the Vite bundle size. Without bundle analysis, large dependencies can be silently added to the build without detection. A `vite-bundle-visualizer` or `rollup-plugin-visualizer` integration should be added so contributors can see the impact of new dependencies before merging.

**Scope of Work:**
- Add `rollup-plugin-visualizer` (pinned version) to `devDependencies`
- Add a `"analyze": "vite build --mode analyze"` script to `package.json`
- Configure the plugin to output `stats.html` in `dist/`
- Add `stats.html` to `.gitignore`

**Out of Scope:**
- Setting bundle size budgets (separate issue)
- Changing the production build configuration

**Acceptance Criteria:**
- [ ] `npm run analyze` produces `dist/stats.html`
- [ ] `stats.html` is gitignored
- [ ] Plugin is listed in `devDependencies` with a pinned version
- [ ] README documents the analyze command

---

## Issue #97: `stellar-payment-platform` missing `CONTRIBUTING.md` with local setup guide

**Summary:**
The repository has no `CONTRIBUTING.md` explaining how to set up the development environment, run tests, and submit pull requests. New contributors spend significant time figuring out the Docker Compose profiles, environment variables, and test commands. A well-structured contributing guide reduces onboarding friction and ensures consistent development practices.

**Scope of Work:**
- Create `CONTRIBUTING.md` at the repository root
- Document: prerequisites (Node, Rust, Docker, stellar CLI), environment setup, running each test suite, and PR conventions
- Reference the Docker Compose profiles table from the README
- Add a link to `CONTRIBUTING.md` from the README

**Out of Scope:**
- Creating issue or PR templates
- Adding a code of conduct (separate)

**Acceptance Criteria:**
- [ ] `CONTRIBUTING.md` exists at the repository root
- [ ] Documents all prerequisites with version requirements
- [ ] Covers running frontend, backend, and contract tests
- [ ] README links to `CONTRIBUTING.md`

---

## Issue #98: `GET /admin/audit-logs` does not paginate results

**Summary:**
The audit log endpoint accepts a `limit` parameter (1–100) but has no `page` or cursor parameter. When there are thousands of audit log entries, operators can only see the most recent 100. Full incident reconstruction requires scrolling through all entries. Cursor-based or offset pagination must be added to allow complete history traversal.

**Scope of Work:**
- Add `page` (default 1) and `limit` (default 50, max 100) query parameters
- Return `meta: { total, page, limit, totalPages }` alongside `data`
- Validate page/limit with the existing pagination schema
- Add tests in `audit-log.test.js` for multi-page retrieval

**Out of Scope:**
- Adding search/filter beyond date range
- Exporting audit logs

**Acceptance Criteria:**
- [ ] `?page=2&limit=10` returns the correct offset slice
- [ ] Response includes `meta.total` and `meta.totalPages`
- [ ] `?limit=200` is clamped to 100
- [ ] Test asserts correct pagination meta for a 25-record dataset with limit=10

---

## Issue #99: `src/utils/tracing.js` OpenTelemetry stub is not wired to any instrumentation

**Summary:**
`src/utils/tracing.js` contains a stub for OpenTelemetry tracing that exports a `tracer` object but is never imported or used by any middleware or service. Distributed traces are never sent to a backend, making the file dead code. The stub should either be removed and replaced with real instrumentation, or kept and integrated with the Express request lifecycle.

**Scope of Work:**
- Decide: integrate real OpenTelemetry HTTP instrumentation or remove the stub
- If integrating: wire `@opentelemetry/instrumentation-http` to `server.js` startup
- If removing: delete `tracing.js` and remove any references
- Add `OTEL_EXPORTER_OTLP_ENDPOINT` to `.env.example` if integrating

**Out of Scope:**
- Integrating with a specific observability vendor
- Adding custom span attributes to all endpoints

**Acceptance Criteria:**
- [ ] `tracing.js` is either removed or fully wired to the Express app
- [ ] If removed: no orphaned imports in `server.js`
- [ ] If integrated: HTTP spans are emitted for every request
- [ ] `.env.example` documents the OTEL variable if applicable

---

## Issue #100: `src/cache/statsCache.js` uses in-memory storage and is not shared across worker processes

**Summary:**
`statsCache.js` stores statistics in a JavaScript `Map`, which is local to a single Node.js process. In a horizontally scaled deployment (multiple Render instances or PM2 cluster), each process has its own isolated cache. Stats requests hitting different instances bypass the cache entirely. The cache must be backed by Redis to be shared across instances.

**Scope of Work:**
- Replace the in-memory `Map` in `statsCache.js` with Redis `GET`/`SET` calls
- Use the existing Redis client from `src/config/redis.js`
- Fall back gracefully to no caching if Redis is unavailable
- Update `stats-cache.test.js` to mock Redis

**Out of Scope:**
- Changing the cache key format
- Adding cache warming logic

**Acceptance Criteria:**
- [ ] Stats cache reads from and writes to Redis
- [ ] Two separate Node processes share the same cache
- [ ] Cache falls back silently when Redis is unavailable
- [ ] Tests mock Redis and assert read/write calls

---

## Issue #101: `src/services/contractService.js` does not validate `CONTRACT_ID` format at startup

**Summary:**
`contractService.js` reads `PAYMENT_ROUTER_CONTRACT_ID` from the environment and uses it to construct Stellar SDK calls. If the env var is missing or not a valid contract ID (`C...` format), the first call to the contract will throw a cryptic Stellar SDK error rather than a clear startup message. The service should validate the contract ID at module load time.

**Scope of Work:**
- Add a `StrKey.isValidContract(id)` check when `contractService.js` is loaded
- Throw a descriptive startup error if the ID is invalid or missing
- Add `PAYMENT_ROUTER_CONTRACT_ID` to `.env.example` with a format comment
- Add a test in `contract-routes.test.js` for the missing-ID path

**Out of Scope:**
- Changing the contract interaction logic
- Validating other env vars in this issue

**Acceptance Criteria:**
- [ ] Server fails to start with a clear error if `PAYMENT_ROUTER_CONTRACT_ID` is invalid
- [ ] Valid contract ID allows normal startup
- [ ] `.env.example` documents the expected `C...` format
- [ ] Test asserts the startup error message

---

## Issue #102: `artillery.yml` load test does not include authentication headers for protected endpoints

**Summary:**
The Artillery load test configuration in `artillery.yml` tests `GET /federation` and `GET /lookup` but excludes authenticated endpoints like `GET /admin/stats/routing` and `GET /admin/audit-logs`. Load behavior under authentication overhead is untested. The Artillery config should include scenarios that exercise admin endpoints with valid API key headers.

**Scope of Work:**
- Add admin scenarios to `artillery.yml` with `X-Api-Key` header
- Add a `before` hook that creates a test API key or reads one from env
- Cover `/admin/stats/routing` and `/admin/audit-logs` in the load scenario
- Document how to run load tests in `README.md`

**Out of Scope:**
- Adding load tests for the Soroban contract
- Running load tests in CI (optional stretch goal)

**Acceptance Criteria:**
- [ ] `artillery.yml` includes at least two admin endpoint scenarios
- [ ] `X-Api-Key` header is set from an env var in the config
- [ ] `artillery run artillery.yml` completes without errors
- [ ] README documents the load test command

---

## Issue #103: `packages/types/src/index.ts` does not export contract error types

**Summary:**
The TypeScript bindings in `packages/types/src/index.ts` export the contract's function signatures but not its error enum variants. Frontend code that calls contract functions cannot type-check errors, leading to `as any` casts throughout the dashboard. The error types from the contract ABI should be exported as a TypeScript union type.

**Scope of Work:**
- Add error type exports to `packages/types/src/index.ts` from the contract ABI
- Export an `ContractError` union type covering all `Error` variants
- Update `npm run generate:bindings` to regenerate if the generator handles errors
- Add a TypeScript compile check for the new types

**Out of Scope:**
- Changes to the contract error definitions
- Frontend error handling implementation

**Acceptance Criteria:**
- [ ] `ContractError` type is exported from `@stellar-tags/payment-router`
- [ ] TypeScript compiles without errors in `packages/types/`
- [ ] At least the known error variants (`InvalidAmount`, `Unauthorized`) are typed
- [ ] `tsconfig.json` `strict` mode passes with the new exports

---

## Issue #104: `src/routes/v1/` route files are not documented inline with JSDoc comments

**Summary:**
The route handlers in `src/routes/v1/` contain no JSDoc comments. Contributors who need to understand endpoint logic must trace through the full middleware stack without any in-code documentation. Each route handler should have a JSDoc block documenting the route path, method, auth requirements, and the shape of request and response.

**Scope of Work:**
- Add JSDoc comments to every handler in `src/routes/v1/`
- Document `@route`, `@access`, `@param`, and `@returns` for each handler
- Ensure comments are consistent with the README endpoint reference
- Run the CI docs spellcheck to verify spelling in comments

**Out of Scope:**
- Generating an OpenAPI spec from JSDoc (separate issue)
- Adding comments to middleware files

**Acceptance Criteria:**
- [ ] Every route handler in `src/routes/v1/` has a JSDoc block
- [ ] JSDoc includes `@route`, `@access`, and `@returns`
- [ ] Spellcheck CI passes with the new comments
- [ ] No existing behavior changed

---

## Issue #105: `payment-dashboard` `Dashboard.jsx` does not memoize expensive derived state calculations

**Summary:**
`Dashboard.jsx` computes several derived values (e.g., transaction totals, balance summaries) on every render without memoization. When parent state updates (e.g., wallet reconnect, network change) cause re-renders, these calculations run unnecessarily. `useMemo` should be applied to any derived values whose inputs have not changed.

**Scope of Work:**
- Identify all derived state calculations in `Dashboard.jsx`
- Wrap each with `useMemo` with the correct dependency array
- Add a test asserting the calculation does not rerun when unrelated state changes
- Profile with React DevTools to confirm reduced re-renders

**Out of Scope:**
- Splitting `Dashboard.jsx` into sub-components
- Backend changes

**Acceptance Criteria:**
- [ ] Derived calculations are wrapped in `useMemo`
- [ ] Re-renders triggered by unrelated state do not recompute derived values
- [ ] Test uses a spy to assert the calculation is called once for stable inputs
- [ ] No visual regression in the dashboard

---

## Issue #106: `src/pagination.js` does not handle negative `total` values from Prisma count queries

**Summary:**
If a Prisma `count()` query returns an unexpected value (e.g., due to a deleted-record race condition), the pagination helper in `src/pagination.js` can produce negative `total` values, resulting in `totalPages: -1` and confusing the client. The helper must clamp `total` to a minimum of `0` and propagate a warning log.

**Scope of Work:**
- Add `total = Math.max(0, total)` in `src/pagination.js` before computing `totalPages`
- Log a warning if the raw total was negative
- Add a test with a negative input asserting the clamped output
- Audit all paginated endpoints to confirm they use the shared helper

**Out of Scope:**
- Fixing the underlying race condition
- Changing the pagination algorithm

**Acceptance Criteria:**
- [ ] `total: -5` input produces `{ total: 0, totalPages: 0 }` output
- [ ] Warning is logged when clamping occurs
- [ ] Test covers the negative and zero cases
- [ ] All paginated endpoints use the shared helper

---

## Issue #107: `src/middleware/deprecation.js` does not log when a deprecated endpoint is called

**Summary:**
The deprecation middleware adds response headers but does not log when a deprecated endpoint is actually used in production. Without logs, it is impossible to measure adoption of deprecated routes or know when it is safe to remove them. Each use of a deprecated endpoint should produce a `warn` log entry with the route path and the caller's correlation ID.

**Scope of Work:**
- Add a `logger.warn('Deprecated endpoint called', { route, correlationId })` in `deprecation.js`
- Use the `logger` from `src/logger.js`
- Add a test asserting the warn log is called for deprecated routes
- Confirm the log does not fire for non-deprecated routes

**Out of Scope:**
- Changing the `Deprecation` or `Sunset` header logic
- Adding deprecation metrics to Prometheus

**Acceptance Criteria:**
- [ ] Calling a deprecated endpoint writes a `warn` log
- [ ] Log includes `route` and `correlationId` fields
- [ ] Non-deprecated routes do not produce the warn log
- [ ] Test spies on `logger.warn` and asserts the call

---

## Issue #108: `src/db-pool-monitor.js` does not alert when pool wait time exceeds a threshold

**Summary:**
`db-pool-monitor.js` collects pool metrics and exposes them via Prometheus gauges but takes no action when the wait queue grows large. A prolonged queue (>10 waiting queries) signals a real-time database bottleneck that should generate a log alert. Adding a threshold check with a `logger.warn` or `logger.error` call creates an immediately actionable signal.

**Scope of Work:**
- Add a periodic check in `db-pool-monitor.js` comparing `waitingQueries` to `DB_POOL_WAIT_ALERT` (default 10)
- Log `logger.warn('DB pool wait queue high', { waiting })` when exceeded
- Log `logger.error(...)` if the queue exceeds `2 * DB_POOL_WAIT_ALERT`
- Add tests in `db-pool-monitor.test.js` for both thresholds

**Out of Scope:**
- Sending alerts to external systems (PagerDuty, etc.)
- Changing the pool size configuration

**Acceptance Criteria:**
- [ ] `waitingQueries > 10` produces a `warn` log
- [ ] `waitingQueries > 20` produces an `error` log
- [ ] `DB_POOL_WAIT_ALERT` env var configures the threshold
- [ ] Tests use mock pool metrics and assert the correct log level

---

## Issue #109: `src/webhookWorker.js` does not emit an event when a webhook is moved to the DLQ

**Summary:**
When a webhook event exhausts all retries and is moved to the dead-letter queue, there is no notification mechanism. Merchants have no way of knowing their webhook is failing without polling the DLQ API. The worker should call a configurable notification hook or emit an activity log event when an item is DLQ'd, so operators are alerted.

**Scope of Work:**
- Log `logger.warn('Webhook moved to DLQ', { webhookId, eventId, lastError })` on DLQ transition
- Optionally: write a `webhook.dlq` activity log entry via `activityService`
- Add a `WEBHOOK_DLQ_NOTIFY_URL` env var: if set, POST a notification payload
- Add tests in `webhook-dlq.test.js` asserting the log and optional notification

**Out of Scope:**
- Building a merchant notification UI
- Implementing email notifications

**Acceptance Criteria:**
- [ ] DLQ transition produces a `warn` log with webhook and event IDs
- [ ] `WEBHOOK_DLQ_NOTIFY_URL` receives a POST with event details when configured
- [ ] Test asserts the warn log on DLQ transition
- [ ] Test asserts the notification POST when the URL is set

---

## Issue #110: `tests/integration/src/main.rs` does not test the refund mechanism end-to-end

**Summary:**
The Soroban integration tests in `tests/integration/src/main.rs` cover successful payment routing and batch payments but do not test the refund flow. An end-to-end test that routes a payment to a recipient without a trustline and then calls `claim_all_refunds` is needed to validate the complete refund lifecycle on a real local Stellar network.

**Scope of Work:**
- Add a test in `main.rs` that sets up a recipient without a trustline
- Route a payment to that recipient and assert the refund ledger is credited
- Call `claim_all_refunds` and assert the sender receives the tokens
- Update `tests/integration/run.sh` to run the new test

**Out of Scope:**
- Adding tests for contract upgrade
- Changing the integration test framework

**Acceptance Criteria:**
- [ ] Integration test for the full refund lifecycle is added
- [ ] Test passes against the local `stellar/quickstart` network
- [ ] `run.sh` includes the new test in its execution plan
- [ ] Test logs show the refund credit and claim amounts

---

## Issue #111: `payment-dashboard` has no error boundary for unhandled React errors

**Summary:**
The React app in `payment-dashboard` has no `ErrorBoundary` component wrapping the main content. An unhandled JavaScript error in any component (e.g., a bad API response shape) causes the entire app to render a blank white screen. A top-level `ErrorBoundary` with a user-friendly fallback UI and a "Reload" button should be added.

**Scope of Work:**
- Create `payment-dashboard/src/ErrorBoundary.jsx` as a class component
- Wrap the main `<App />` in `main.jsx` with `<ErrorBoundary>`
- Display a fallback UI with an error message and a page reload button
- Add a test triggering an error and asserting the fallback renders

**Out of Scope:**
- Integrating error reporting services (Sentry, etc.)
- Per-component error boundaries for sub-sections

**Acceptance Criteria:**
- [ ] A thrown error in a child component shows the fallback UI
- [ ] Fallback includes a "Reload page" button
- [ ] White screen of death is replaced by the error boundary UI
- [ ] Test asserts fallback renders when a child throws

---

## Issue #112: `src/services/activityService.js` does not support bulk activity log inserts

**Summary:**
The activity service writes one log entry per call using a single Prisma `create`. Operations that produce multiple activity events (e.g., bulk username imports, batch deregistrations) make N sequential inserts, adding unnecessary latency. The service should accept an array of events and use `createMany` when multiple entries are provided.

**Scope of Work:**
- Add a `logMany(events: ActivityEvent[])` method to `activityService.js`
- Use Prisma `createMany` with `skipDuplicates: false` internally
- Update callers that log multiple events in a loop to use `logMany`
- Add a test asserting `createMany` is called instead of N `create` calls

**Out of Scope:**
- Changing the single-event `log` method signature
- Adding activity event deduplication

**Acceptance Criteria:**
- [ ] `logMany([...])` inserts all events in one Prisma call
- [ ] Existing `log(event)` method still works
- [ ] Test mocks Prisma `createMany` and asserts it is called once for N events
- [ ] No performance regression for single-event logging

---

## Issue #113: `docs/grafana-dashboard.json` panels use hardcoded datasource UID

**Summary:**
The Grafana dashboard JSON in `docs/grafana-dashboard.json` uses a hardcoded datasource UID (`abc123` or similar) that only works in the original developer's Grafana instance. Importing the dashboard into any other Grafana instance requires manually updating every panel's datasource. The dashboard should use a datasource variable (`${DS_PROMETHEUS}`) so it works in any environment on import.

**Scope of Work:**
- Replace hardcoded datasource UIDs in `grafana-dashboard.json` with `${DS_PROMETHEUS}`
- Add a `__inputs` section defining the `DS_PROMETHEUS` input
- Test by importing the JSON into a fresh Grafana instance
- Update `docs/grafana-dashboard.md` with import instructions

**Out of Scope:**
- Adding new dashboard panels
- Changing the Prometheus metric names

**Acceptance Criteria:**
- [ ] All panel datasources use `${DS_PROMETHEUS}` template variable
- [ ] Dashboard JSON includes `__inputs` section
- [ ] Import into a fresh Grafana instance works without manual edits
- [ ] Docs describe the import process

---

## Issue #114: `src/middleware/requireJson.js` does not reject requests with `Content-Type: text/plain` on JSON endpoints

**Summary:**
The `requireJson` middleware checks for `application/json` content type on mutation endpoints, but some HTTP clients send `text/plain` with a JSON body (a common browser CSRF pattern). The middleware should reject all non-JSON content types with `415 UNSUPPORTED_MEDIA_TYPE` to prevent this class of CSRF bypass.

**Scope of Work:**
- Update `requireJson.js` to use `req.is('application/json')` instead of header string matching
- Ensure `text/plain` and `application/x-www-form-urlencoded` are rejected
- Return `ApiError('UNSUPPORTED_MEDIA_TYPE', ...)` with `415` status
- Add tests in a new or existing test file for each rejected content type

**Out of Scope:**
- Adding CSRF token validation
- Changing the error code for other request issues

**Acceptance Criteria:**
- [ ] `Content-Type: text/plain` on `POST /register` returns `415`
- [ ] `Content-Type: application/x-www-form-urlencoded` returns `415`
- [ ] `Content-Type: application/json` is accepted
- [ ] Test covers all three content type scenarios

---

## Issue #115: `src/webhookWorker.js` webhook secret is stored in plain text in the `Webhook` table

**Summary:**
The webhook signing secret is stored in plain text in the `Webhook.secret` column. If the database is compromised, all webhook secrets are exposed, allowing an attacker to forge valid HMAC signatures for any merchant's endpoint. Secrets should be encrypted at rest using AES-256-GCM with a server-side `WEBHOOK_SECRET_KEY` env var before storage.

**Scope of Work:**
- Add `WEBHOOK_SECRET_KEY` (32-byte hex) env var for encryption
- Add encrypt/decrypt utilities in `src/utils/` using Node.js `crypto`
- Encrypt secrets before inserting into `Webhook.secret`
- Decrypt before use in `webhookWorker.js`
- Add a migration to re-encrypt existing plain-text secrets (or document manual step)
- Update `webhook-signature.test.js`

**Out of Scope:**
- Key rotation mechanism
- Changing the HMAC algorithm

**Acceptance Criteria:**
- [ ] Webhook secrets are stored as encrypted ciphertext in the DB
- [ ] HMAC is computed using the decrypted secret
- [ ] Test verifies a forged secret does not match
- [ ] `.env.example` documents `WEBHOOK_SECRET_KEY`

---

## Issue #116: `src/schemas/index.js` does not enforce `username` character allowlist

**Summary:**
The username field in the registration schema is validated for length but not for allowed characters. Usernames containing Unicode, control characters, or special characters like `<`, `>`, and `&` can cause XSS vulnerabilities when rendered in HTML contexts and break federation parsing. The schema must enforce an alphanumeric-plus-hyphen allowlist.

**Scope of Work:**
- Add a `.regex(/^[a-z0-9-]+$/i)` constraint to the `username` field in `registerBodySchema`
- Add a min length of 3 and max length of 30 if not already enforced
- Return `422 VALIDATION_FAILED` for non-conforming usernames
- Add tests in `schemas.test.js` for each disallowed character class

**Out of Scope:**
- Supporting internationalized usernames (IDN)
- Changes to the frontend form validation

**Acceptance Criteria:**
- [ ] `username: "alice<script>"` returns `422`
- [ ] `username: "alice-bob"` passes validation
- [ ] `username: "a"` (too short) returns `422`
- [ ] Tests cover alphanumeric, hyphen, special char, and length cases

---

## Issue #117: `.github/workflows/deploy-testnet.yml` does not require manual approval before deploying

**Summary:**
The testnet deploy workflow in `deploy-testnet.yml` triggers automatically on push to `main` without a manual approval gate. An accidental or unreviewed push to main can trigger a testnet deployment that breaks the integration test environment for the whole team. The workflow should require a manual approval from a designated reviewer before running the deploy steps.

**Scope of Work:**
- Add a GitHub `environment` with a required reviewer to `deploy-testnet.yml`
- Create a `testnet` environment in the repository settings (document the step)
- Move the deploy steps after the environment approval gate
- Update `DEPLOYMENT_CHECKLIST.md` to include the approval step

**Out of Scope:**
- Adding mainnet deployment workflow
- Changing the deploy script

**Acceptance Criteria:**
- [ ] Pushing to `main` requires manual approval before deploy steps run
- [ ] The `environment: testnet` key is in the workflow YAML
- [ ] `DEPLOYMENT_CHECKLIST.md` documents the approval requirement
- [ ] Deploy steps still run correctly after approval

---

## Issue #118: `src/utils/jwt.js` does not validate `iss` and `aud` claims on token verification

**Summary:**
The JWT verification in `src/utils/jwt.js` validates the signature and expiry but does not check `iss` (issuer) or `aud` (audience) claims. A token issued by a different service with the same signing key would pass verification. The verifier must check that `iss === "stellar-payment-platform"` and `aud === "api"` to prevent cross-service token reuse.

**Scope of Work:**
- Add `issuer` and `audience` options to the `jwt.verify()` call in `jwt.js`
- Set `iss: "stellar-payment-platform"` and `aud: "api"` in token creation
- Return `UNAUTHENTICATED` error if claims do not match
- Update `jwt-utils.test.js` to assert claim validation

**Out of Scope:**
- Adding refresh token logic
- Changing the token expiry duration

**Acceptance Criteria:**
- [ ] Token without `iss` claim is rejected with `UNAUTHENTICATED`
- [ ] Token with wrong `aud` is rejected
- [ ] Valid token with correct claims passes verification
- [ ] Test covers mismatched `iss`, mismatched `aud`, and valid token

---

## Issue #119: `payment-dashboard` `HelpPage.jsx` code examples are not syntax-highlighted

**Summary:**
The `HelpPage.jsx` displays webhook verification examples copied from `docs/webhook-signature-verification.md` but renders them as plain `<pre>` blocks without syntax highlighting. This reduces readability for developers integrating the API. A lightweight syntax highlighter (e.g., `prism-react-renderer`) should be added to display code blocks with language-appropriate coloring.

**Scope of Work:**
- Add `prism-react-renderer` (pinned version) to `payment-dashboard/package.json`
- Replace `<pre>` blocks in `HelpPage.jsx` with the highlighted component
- Support at least `javascript`, `python`, and `go` languages
- Ensure the highlighter does not significantly increase bundle size

**Out of Scope:**
- Adding a full Markdown renderer
- Changes to the documentation content

**Acceptance Criteria:**
- [ ] Code blocks in `HelpPage.jsx` display with syntax coloring
- [ ] Three languages (JS, Python, Go) are highlighted correctly
- [ ] Bundle size increase is documented (e.g., +15 KB gzipped)
- [ ] Accessibility: highlighted code meets WCAG 4.5:1 contrast ratio

---

## Issue #120: `src/services/statsService.js` does not filter out soft-deleted users from volume aggregation

**Summary:**
The routing stats aggregation in `statsService.js` counts payment intents associated with soft-deleted users toward the volume and count totals. This inflates the statistics with data from accounts that have been removed. The aggregation query must join with the `User` table and exclude records where `deletedAt IS NOT NULL`.

**Scope of Work:**
- Update the Prisma query in `statsService.js` to join/filter on `User.deletedAt IS NULL`
- Add a test in `admin-stats-routing.test.js` with a soft-deleted user's payment intents
- Verify the deleted user's data is excluded from the aggregate
- Check the same filter is applied in the `assetCode` filter path

**Out of Scope:**
- Changing the stats response schema
- Purging soft-deleted payment intents

**Acceptance Criteria:**
- [ ] Payment intents from soft-deleted users are excluded from stats
- [ ] Test creates a soft-deleted user with intents and asserts exclusion
- [ ] Stats totals decrease by the deleted user's contribution in the test
- [ ] Existing stats tests pass

---

## Issue #121: `src/middleware/asyncHandler.js` swallows errors that are not instances of `Error`

**Summary:**
The `asyncHandler` middleware in `src/middleware/asyncHandler.js` wraps route handlers and calls `next(err)` on rejection. However, if a handler throws a plain string or object instead of an `Error` instance (a common JavaScript mistake), the error is passed to `next` as-is, bypassing the typed error handler and leaking the raw value in the response. The middleware should normalize non-Error throws to `ApiError('INTERNAL_ERROR', ...)`.

**Scope of Work:**
- Update `asyncHandler.js` to check `err instanceof Error`
- Wrap non-Error values in `new ApiError('INTERNAL_ERROR', 'Unexpected error')` and log the original
- Add tests for throwing a string, an object, and an `ApiError`
- Ensure the original thrown value is logged under `reference_id`

**Out of Scope:**
- Changing the global error handler
- Adding new error codes

**Acceptance Criteria:**
- [ ] `throw "something went wrong"` returns `500 INTERNAL_ERROR`
- [ ] `throw { code: 'custom' }` is wrapped and returns `500`
- [ ] `throw new ApiError(...)` passes through unchanged
- [ ] Test covers all three throw types

---

## Issue #122: `payment-dashboard` does not implement Content Security Policy via meta tag for Vite builds

**Summary:**
The index.html served by Vite does not include a `<meta http-equiv="Content-Security-Policy">` tag. Without a CSP, the browser allows inline scripts, external resources, and eval, making the app vulnerable to XSS. A strict CSP meta tag restricting `script-src`, `style-src`, and `connect-src` should be added to `index.html`.

**Scope of Work:**
- Add a CSP meta tag to `payment-dashboard/index.html`
- Configure `script-src 'self'`, `style-src 'self' 'unsafe-inline'` (for Vite HMR), `connect-src 'self' <API_BASE>`
- Use `VITE_API_BASE` to parameterize `connect-src` in the Vite config
- Verify no inline scripts or `eval` usage is blocked

**Out of Scope:**
- Server-side CSP headers (those are set by the backend)
- Nonce-based CSP (complex, separate issue)

**Acceptance Criteria:**
- [ ] `index.html` includes a `Content-Security-Policy` meta tag
- [ ] `script-src 'self'` is set
- [ ] Browser console shows no CSP violations in development mode
- [ ] `connect-src` includes the API base URL

---

## Issue #123: `stellar-payment-platform` package.json jest coverage thresholds are set too low

**Summary:**
The Jest `coverageThreshold` in `stellar-payment-platform/package.json` is configured but the thresholds are below what the current test suite actually achieves, meaning the floor check never triggers in practice. The thresholds should be raised to match or slightly exceed current coverage to ensure any new code without tests fails CI.

**Scope of Work:**
- Run `npm run test:coverage` to get current branch/line/function coverage
- Update `coverageThreshold` in `package.json` to current values minus 2 percentage points as a floor
- Document the process for raising thresholds in `CONTRIBUTING.md`
- Add a CI check note about coverage in the README

**Out of Scope:**
- Writing new tests to increase coverage
- Changing the test framework

**Acceptance Criteria:**
- [ ] `npm run test:coverage` passes with the new thresholds
- [ ] Deleting a test file causes the CI coverage check to fail
- [ ] `CONTRIBUTING.md` explains how to update thresholds
- [ ] Current coverage percentage is documented

---

## Issue #124: `src/routes/v1/` missing route for `DELETE /webhooks/:id` to deregister a webhook

**Summary:**
The system allows creating webhooks via the registration flow and lists them, but there is no `DELETE /webhooks/:id` endpoint for deregistering an individual webhook. Merchants who need to remove a compromised or obsolete webhook must contact support or delete via database access. A delete endpoint with ownership verification must be added.

**Scope of Work:**
- Add `DELETE /webhooks/:id` route to `src/routes/v1/`
- Verify webhook ownership using `ownershipService.js` (signature required)
- Soft-delete the webhook and write a `webhook.deleted` activity log entry
- Add tests in a new or existing webhook test file

**Out of Scope:**
- Hard-delete or bulk-delete endpoints
- Admin override for webhook deletion

**Acceptance Criteria:**
- [ ] `DELETE /webhooks/:id` with valid signature deletes the webhook
- [ ] Attempting to delete another user's webhook returns `403 FORBIDDEN`
- [ ] Webhook no longer receives deliveries after deletion
- [ ] Activity log contains a `webhook.deleted` entry

---

## Issue #125: `docs/api-collections/Stellar-Tags-API.postman_collection.json` is missing admin and activity endpoints

**Summary:**
The Postman collection in `docs/api-collections/` covers only the core endpoints (`/federation`, `/register`, `/lookup`). The admin export, routing stats, audit logs, activity trail, and metrics endpoints are absent. Contributors and integration partners using the collection for testing cannot test these endpoints without building requests from scratch. The collection must be updated to include all documented endpoints.

**Scope of Work:**
- Add request entries for: `GET /admin/export`, `GET /admin/stats/routing`, `GET /admin/audit-logs`, `GET /users/:username/activity`, `GET /metrics`, `GET /health`
- Include example request headers (e.g., `x-api-key`, `X-Stellar-Signature`) with placeholder values
- Add response examples for `200` and common error codes
- Test the collection by importing into Postman and running the collection runner

**Out of Scope:**
- Adding automated test scripts to the Postman collection
- Publishing the collection to the Postman public workspace

**Acceptance Criteria:**
- [ ] Postman collection includes all 10+ documented endpoints
- [ ] Each request has example headers and query params
- [ ] Collection imports into Postman without errors
- [ ] README links to the collection and describes how to use it
