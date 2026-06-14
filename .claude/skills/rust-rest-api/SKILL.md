---
name: rust-rest-api
description: |
  Rules for the Rust REST API backend: layered architecture (handler → service → db),
  DTOs, response envelope, error handling, and full test coverage (unit + integration + HURL).
  TRIGGER when: starting work on any task in the `backend/` directory of this repo,
  including the first scaffolding tasks before files exist; editing files under
  backend/src/{handlers,services,db,dto}/ or backend/tests/; writing or modifying
  axum/sqlx code; adding HURL tests under tests/hurl/; reviewing a backend PR;
  designing a new endpoint or repository function; applying tasks from an OpenSpec
  change whose tasks.md mentions backend Rust files. Invoke before writing the
  first line of Rust in a session.
  SKIP: frontend code, infrastructure-only changes (Dockerfiles, Compose, Traefik
  config), migrations-only edits, or documentation changes that don't touch
  handler/service/db code.
---

# Rust REST API — Rules & Guidance

## Module Layout

Code is organised by **layer**, not by domain. Every handler lives in `src/handlers/`, every service in `src/services/`, every DB function in `src/db/`. Files within each layer are named by domain/resource.

```
backend/src/
├── main.rs              # server entry point, router wiring
├── app_state.rs         # AppState struct
├── api_key.rs           # SHARED — API-key generate/hash primitives (layer-agnostic)
├── config.rs            # env-var loading, fail-fast
├── crypto.rs            # SHARED — AES-GCM token encryption + session-JWT sign/verify
├── error.rs             # AppError enum + IntoResponse
├── response.rs          # ApiResponse<T> envelope type
├── session.rs           # Session + SessionStore
│
├── audit/               # CROSS-CUTTING capability — event catalogue + its own SQL
│   └── mod.rs
├── permissions.rs       # CROSS-CUTTING capability — map-permission resolver + its own SQL
│
├── handlers/            # HANDLER LAYER — one file per route group
│   ├── mod.rs
│   ├── auth.rs          # /auth/* routes: login, callback, logout, add_character
│   ├── cookie.rs        # session-cookie helpers (HTTP concern → handler layer)
│   ├── middleware.rs    # auth extractors + cookie-refresh middleware
│   └── api/
│       └── v1/
│           ├── mod.rs
│           ├── keys.rs
│           ├── me.rs
│           ├── characters.rs
│           └── account.rs
│
├── services/            # SERVICE LAYER — one file per domain
│   ├── mod.rs
│   ├── auth.rs          # SSO flow, session management
│   └── account.rs       # account + character business logic
│
├── db/                  # DATABASE LAYER — one file per resource
│   ├── mod.rs           # connect() + migration runner + DbError
│   ├── accounts.rs
│   ├── characters.rs
│   └── api_keys.rs
│
├── dto/                 # DTOs — one file per resource or handler group
│   └── *.rs
│
└── esi/                 # ESI client helpers (not a layer — external API client)
    ├── mod.rs           # EsiMetadata + discover()
    └── public_info.rs
```

**Rules enforced by this layout:**
- Never add a handler function outside `src/handlers/`.
- Never add a service function outside `src/services/`.
- Never add a DB function outside `src/db/` — except inside a blessed cross-cutting capability module (below).
- **Layer-agnostic primitives live at the crate root**, not inside a layer: `crypto.rs` and `api_key.rs` are consumed by handlers, services, *and* db code, so placing them in any layer would force upward imports. If a helper is needed by more than one layer, it belongs at the root alongside `session.rs`/`response.rs`.
- Genuinely HTTP-shaped helpers (`cookie.rs`, `middleware.rs`) stay in `src/handlers/` — they are handler-layer concerns.

**Cross-cutting capability modules.** `audit/` and `permissions.rs` own their SQL even though they are not in `src/db/`. This is deliberate: the audit catalogue's invariants (event-type stability, write-time actor/target snapshots) are inseparable from its insert, and the permission resolver's rules (owner bypass, deny veto, most-permissive-wins) are inseparable from its query. Splitting either across `db/` would hurt cohesion without improving safety. A new module qualifies for this exception only when its queries and its domain rules are this entangled — a plain CRUD resource never does.

---

## Architecture: The Only Permitted Flow

```
HTTP Request
    │
    ▼
Handler  (src/handlers/)
    │  validates input, calls service, returns DTO wrapped in envelope
    ▼
Service  (src/services/)
    │  owns business logic, calls db layer
    ▼
DB / Repository  (src/db/)
    │  raw SQL or ORM queries, returns domain types
    ▼
Database
```

**This flow is strictly one-directional and may not be broken:**

- Handlers **must not** call `db` functions directly.
- Services **must not** import `axum` or any HTTP framework types.
- DB functions **must not** contain business logic.
- No layer may import from a layer above it.

**Enforcement is mechanical, not review discipline.** `backend/tests/layering.rs` scans the source tree and fails on: handlers referencing `crate::db`, db files importing `crate::handlers`/`crate::services`, and services importing `crate::handlers` or `axum`. Exceptions are an explicit, commented allowlist in that file. The one standing exception: `src/handlers/middleware.rs` — the auth extractors resolve accounts/keys/blocks directly against the db as a pre-handler concern, and the fail-closed auth-coverage tests pin that behaviour. Add a new exception only with a justification comment; never weaken the test to get a build green.

---

## Handler Rules

- Handlers live in `src/handlers/`.
- Each handler **must** accept injected state (e.g., `State<AppState>`) — no globals.
- Handlers **must** call exactly one service function per logical operation.
- Handlers **must** return a DTO (not a DB model) wrapped in the standard envelope.
- Validation of incoming request bodies happens in the handler before calling the service.

```rust
// CORRECT
async fn create_user(
    State(state): State<AppState>,
    Json(body): Json<CreateUserRequest>,
) -> Result<Json<ApiResponse<UserDto>>, AppError> {
    body.validate()?;
    let user = state.user_service.create_user(body).await?;
    Ok(Json(ApiResponse::data(user)))
}

// WRONG — handler calling db directly
async fn create_user(State(state): State<AppState>, ...) {
    let user = db::users::insert(&state.db, ...).await?; // ❌ layering.rs fails
}
```

---

## Service Rules

- Services live in `src/services/`.
- Services **must not** import `axum`, or any HTTP framework types (enforced by `tests/layering.rs`).
- Services own all business logic: validation that depends on persisted state, orchestration of multiple DB calls, etc.
- If a DB method can be slightly extended (e.g., return one extra column, add a `RETURNING` clause) to avoid a second round-trip, **extend the DB method** — do not write a second DB function. **Exception:** if extending would force unrelated callers to fetch significantly more data (large `TEXT` / `BYTEA` columns, a wide join), add a dedicated function instead. The rule is "one round-trip per operation", not "every caller pays for every reader's needs".
- Services return domain types or DTOs; never raw DB row types.

```rust
// CORRECT — extend the query, don't add a second db call
pub async fn activate_user(pool: &PgPool, id: Uuid) -> Result<UserDto, AppError> {
    // The db fn returns the updated row — no second fetch needed
    let user = db::users::activate(pool, id).await?;
    Ok(UserDto::from(user))
}

// WRONG — two db calls when one would do
let _ = db::users::activate(pool, id).await?;      // ❌
let user = db::users::find_by_id(pool, id).await?; // ❌ unnecessary second call
```

---

## DB / Repository Rules

- DB functions live in `src/db/`. One file per resource (`accounts.rs`, `characters.rs`, `api_keys.rs`). Never nest further.
- Each function maps to a query or a small, cohesive set of queries.
- **Before adding a new function**, check whether an existing one can be extended (e.g., add a `RETURNING` clause, join an extra table) to satisfy the new requirement.
- DB functions return domain model structs (`User`, `Order`, …) — never raw query row types exposed outside `src/db/`.
- No business logic inside DB functions. A DB function that takes a `status: &str` and validates it is wrong — validation belongs in the service.
- Avoid byte-identical pool/tx twin functions; prefer one function generic over `impl PgExecutor<'_>`, or keep only the tx variant when every caller is transactional.

---

## DTOs

- DTOs live in `src/dto/` (or co-located per feature — be consistent).
- Every handler response **must** use a DTO, never a DB model.
- Implement `From<DbModel> for Dto` — do not map fields inline in handlers or services.
- A DTO is an **explicit allowlist** of safe-to-serialise fields. Treat every field as an intentional decision to expose it.
- Never use `#[serde(flatten)]` to fold a DB model into a DTO — that smuggles every field of the DB type into the wire format, including any added later.
- Never `#[derive(Serialize)]` directly on a DB model (i.e. on the struct returned from `src/db/`). Serialisation is a DTO responsibility; DB models stay internal.
- Do not use `#[serde(skip_serializing_if = ...)]` on a sensitive field as a guard — conditional skip is not allowlisting; just don't include the field in the DTO.

```rust
#[derive(Serialize)]
pub struct UserDto {
    pub id: Uuid,
    pub email: String,
    pub created_at: DateTime<Utc>,
    // ← password_hash is NOT here
}

impl From<User> for UserDto {
    fn from(u: User) -> Self {
        Self { id: u.id, email: u.email, created_at: u.created_at }
    }
}
```

---

## API Response Envelope

All endpoints **except** `/api/health` must return:

```json
{ "data": <payload> }
```

For lists:
```json
{ "data": [ … ] }
```

For single items:
```json
{ "data": { … } }
```

### Envelope type

```rust
#[derive(Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub data: T,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn data(payload: T) -> Self {
        Self { data: payload }
    }
}
```

### `/api/health` exception

The health endpoint (route: `/api/health`) returns its own flat structure — **no envelope**:

```json
{ "status": "ok", "version": "1.2.3", "commit": "…", "components": [ … ] }
```

Do not wrap it. Do not apply `ApiResponse` to it. (This is the `api-contract` spec's documented carve-out; the route is `/api/health`, not `/api/healthz`.)

---

## Error Handling

- Define a single `AppError` enum in `src/error.rs`, with `IntoResponse` converting it at the handler boundary.
- **Services return `AppError` directly.** There is no separate `ServiceError` type — at this codebase's size a parallel enum is pure ceremony. The layering rule that matters is behavioural: services never *construct* HTTP responses or status codes; they return `AppError` variants and the `IntoResponse` impl owns the mapping.
- The db layer returns `anyhow::Result` or `DbError` (`src/db/mod.rs`), which distinguishes constraint violations (`UniqueViolation`, by SQLSTATE/constraint inspection — never by matching error-message text). Services translate `DbError` into typed `AppError::Conflict(...)` / `BadRequest` exactly at the call sites that know which constraint means what; everything else defaults to `AppError::Internal`.
- Never use `.unwrap()` or `.expect()` in handler, service, or DB code. **Enforced**: `[lints.clippy] unwrap_used / expect_used = "warn"` in `backend/Cargo.toml` (CI's `-D warnings` makes them denies), with `clippy.toml` `allow-unwrap-in-tests / allow-expect-in-tests = true` and crate-level allows in `tests/*.rs`. A provably-infallible case in non-test code may carry a narrowly-scoped `#[allow(clippy::expect_used)]` **with a comment proving why it cannot panic** — an allow without a proof comment is a review-blocker.

---

## Testing Requirements

### Test environment

Backend tests run against the contributor's **local Postgres** (no Docker harness). `cargo test` picks up `DATABASE_URL` from `backend/.cargo/config.toml`; the role needs `CREATEDB` and must own the base database so `#[sqlx::test]` can spawn its per-test DBs and bookkeeping schema. Setup steps live in `CONTRIBUTING.md`.

`sqlx::query!` macros validate against the live DB at compile time. To keep `cargo build` working without a database (and for CI), the repo ships a committed `backend/.sqlx/` offline cache. **After adding, removing, or changing any `sqlx::query!` invocation, regenerate the cache with `cargo sqlx prepare -- --all-targets` from `backend/` and commit the diff.** The `--all-targets` flag is required so test-only `sqlx::query!` invocations (under `#[cfg(test)]` and in integration tests) are also cached — without it the cache drifts and CI fails on the next run. CI runs `cargo sqlx prepare --check -- --all-targets` and fails on drift.

### Unit Tests — 100% coverage of every non-trivial function

Unit-test coverage is **not** limited to the service layer. Every function with meaningful behaviour gets a unit test — handlers, services, DB functions, and helpers alike. The only exclusions are trivial glue (one-line `From` impls, a constructor that just assigns fields, a handler that does literally nothing but `service.call().await`). If a function has a branch, a transformation, a validation, or an error path, it needs a test.

Tests live in `#[cfg(test)]` modules within the file they cover (preferred — keeps the test next to the code), or in a sibling `tests.rs` for that module.

**This codebase tests against real per-test databases, not mocks.** `#[sqlx::test]` hands every test its own freshly-migrated database; handler, service, and db tests all use it. There is no `mockall` / trait-double infrastructure — do not introduce one. Real-DB tests catch SQL drift, constraint behaviour, and transaction semantics that doubles structurally cannot. External HTTP (ESI, SSO) is the one boundary that is mocked — with `wiremock` at the network level, never with traits.

**Coverage targets per layer:**

| Layer | What to test | How |
|---|---|---|
| Handler | request parsing, validation dispatch, envelope shape, error → status mapping | `#[sqlx::test]` + build the router/extractor against the test pool; drive with real requests (`tower::ServiceExt::oneshot`) |
| Service | business logic, orchestration, transactional behaviour, error translation | `#[sqlx::test]` — call the service against the test pool; `wiremock` for any ESI interaction |
| DB | SQL correctness, row-to-domain mapping, constraint behaviour, transaction semantics | `#[sqlx::test]` — unit-level scope, one function per test |
| Helper / pure functions | every branch and edge case | direct calls — no DB, no mocks |
| DTO `From` impls | only if the mapping is non-trivial (computed fields, conditional inclusion, redaction) | direct |
| Error → response mapping | every `AppError` variant maps to the documented status & body | construct the variant, call `into_response` |

**Service example — real DB, mocked ESI boundary:**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test]
    async fn block_tears_down_owning_account(pool: PgPool) {
        let (account, character) = seed_account_with_character(&pool).await;
        block_character(&pool, admin_id, character.eve_character_id, None, None, None)
            .await
            .unwrap();
        assert!(sessions_for(&pool, account).await.is_empty());
    }
}
```

**DB example — real test DB, one function under test:**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test]
    async fn insert_duplicate_email_returns_unique_violation(pool: PgPool) {
        insert(&pool, NewUser { email: "a@b.com".into() }).await.unwrap();
        let err = insert(&pool, NewUser { email: "a@b.com".into() }).await.unwrap_err();
        assert!(matches!(err, DbError::UniqueViolation { .. }));
    }
}
```

**Helper example — pure function, no setup:**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_email_lowercases_and_trims() {
        assert_eq!(normalize_email("  A@B.COM  "), "a@b.com");
    }
}
```

Note the boundary with integration tests: a *unit* DB test exercises one function in isolation; an *integration* test exercises a full handler→service→db request. Both use `#[sqlx::test]`; the distinguishing factor is scope, not tooling.

### Guard tests — fail-closed architecture checks

Two standing test suites enforce structure; keep them passing and extend them when the structures they guard grow:

- `tests/layering.rs` — the layer-boundary scan described under Architecture.
- The auth-coverage tests — every `/api/v1/*` route must reject unauthenticated callers (and `/api/v1/admin/*` must 401/403 correctly); route lists in `lib.rs` are kept in lockstep with the router.

### Integration Tests — 100% coverage of handler→service→db paths

- Live in `tests/` at the project root.
- Use a real (test) database via `#[sqlx::test]`.
- Every handler must be exercised end-to-end at least once.
- Test both happy paths and key error paths (not found, validation failure, conflict).

```rust
use tower::ServiceExt; // brings `.oneshot()` into scope on `axum::Router`

#[sqlx::test]
async fn test_create_user_returns_dto_envelope(pool: PgPool) {
    let app = build_test_app(pool);
    let resp = app.oneshot(post_json("/api/users", json!({"email": "a@b.com"}))).await.unwrap();
    assert_eq!(resp.status(), 201);
    let body: Value = parse_body(resp).await;
    assert!(body["data"]["id"].is_string());
    assert!(body["data"].get("password_hash").is_none()); // DTO, not DB model
}
```

### HURL Tests — 100% coverage of HTTP endpoints

- Every endpoint must have at least one HURL test in `tests/hurl/`.
- HURL tests are the source of truth for the HTTP contract (status codes, headers, response shape).
- Name files after the resource: `users.hurl`, `orders.hurl`, `health.hurl`.
- Test the envelope shape explicitly.

`tests/hurl/users.hurl` — envelope-shape assertion on a wrapped endpoint:

```hurl
POST http://localhost:8080/api/users
Content-Type: application/json
{
  "email": "test@example.com",
  "password": "secret123"
}

HTTP 201
[Asserts]
jsonpath "$.data.id" isString
jsonpath "$.data.email" == "test@example.com"
jsonpath "$.data" not exists "password_hash"
```

`tests/hurl/health.hurl` — explicit assertion that `/api/health` has **no** envelope:

```hurl
GET http://localhost:8080/api/health

HTTP 200
[Asserts]
jsonpath "$.status" == "ok"
jsonpath "$" not exists "data"
```

Two HURL requests in the same file are separated by a blank line — the file reads top-to-bottom. (Do not put a `---` separator inside a HURL file; HURL doesn't need one and it confuses markdown renderers when the snippet is embedded.)

---

## Checklist Before Committing

- [ ] Handler does not call db directly (`tests/layering.rs` passes)
- [ ] Service does not import HTTP types (`tests/layering.rs` passes)
- [ ] Shared primitives placed at crate root, not inside a layer
- [ ] DB function was extended rather than duplicated where possible; no pool/tx twins
- [ ] Response uses a DTO, not a DB model
- [ ] Response is wrapped in `ApiResponse` envelope (except `/api/health`)
- [ ] `cargo fmt` must be executed
- [ ] Unit test for every non-trivial function — real `#[sqlx::test]` DBs, `wiremock` for ESI, no trait mocks
- [ ] Integration test for every handler (happy + key error paths)
- [ ] HURL test for every endpoint
- [ ] No `.unwrap()` / `.expect()` in non-test code (clippy-enforced); any `#[allow]` carries a cannot-panic proof comment
- [ ] `AppError` handles all error cases; no ad-hoc `StatusCode` returns in handlers
- [ ] `cargo clippy --all-targets -- -D warnings` passes
