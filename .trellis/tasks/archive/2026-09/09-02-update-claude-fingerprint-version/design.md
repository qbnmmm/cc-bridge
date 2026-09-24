# Technical Design: Claude Code 2.1.258 Client Profile Alignment

## 1. Scope and sequencing

The work has two ordered stages inside one task:

1. Evidence stage: capture the exact official Claude Code 2.1.258 application and transport profile.
2. Alignment stage: update cc-bridge only for differences proven by the capture or static binary evidence.

The capture is a planning/research gate, not a production code change. TLS/craftls changes remain conditional; no transport code is modified merely because the official executable is Bun-packaged.

## 2. Capture architecture

### 2.1 Baseline executable

Run the exact binary:

`/Users/qiubingnan/.cac/versions/2.1.258/claude`

Do not require a local Claude.ai/OAuth login. The active CAC environment already provides `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN` in its isolated settings; use those existing relay credentials without printing or copying their values.

Read the existing relay base URL/token into the child process environment without printing them or writing them to a temporary file. Run with an empty temporary `CLAUDE_CONFIG_DIR`, so no fingerprint hooks, telemetry spoofing, plugins or project settings are loaded. Do not attempt `/login`, OAuth token extraction or direct authenticated access to Anthropic.

### 2.2 Request scenarios

Run from a temporary empty working directory with MCP/plugins minimized:

1. One noninteractive default Sonnet request asking for the exact text `OK`.
2. One noninteractive Haiku request asking for the exact text `OK`.

Do not enable 1M context, fast mode, remote control, tools, or concurrent sessions.

### 2.3 Application-layer capture

Use a loopback `mitmdump` process and a temporary addon under `/tmp`.

The addon must redact before persistence:

- Preserve request method, host, path, HTTP version, header names/order/casing and allowlisted protocol values.
- Preserve values for UA, beta, Anthropic/Stainless version, content type/encoding and feature headers.
- Replace Authorization, cookies, tokens, emails, account/org UUIDs, device/session/request IDs and prompt/response text with typed placeholders.
- Preserve JSON key structure, value types, array lengths and selected non-sensitive enum/boolean values; do not persist raw message or system text.
- Do not persist response bodies.

The live capture is limited to the official 2.1.258 client calling the existing relay, primarily `/v1/messages` and any request the client naturally sends to the configured relay base URL. It cannot establish the exact Claude.ai-subscriber `/api/oauth/usage`, metrics or event behavior because there is no local OAuth login; those paths must be analyzed from the 2.1.258 binary/control flow and from cc-bridge's pure rewrite output, not fabricated as live evidence.

### 2.4 Transport-layer capture

Record the official client-to-local-proxy-to-existing-relay connection. The ClientHello still identifies the 2.1.258 client stack, but SNI, negotiated ALPN and server-dependent HTTP/2 behavior describe the relay destination rather than a direct `api.anthropic.com` connection. Extract, where available:

- TLS cipher and extension ordering
- supported groups and signature algorithms
- SNI/ALPN shape
- JA3/JA4-compatible material
- negotiated protocol
- HTTP/2 SETTINGS and initial stream/header ordering

If exact HTTP/2 or JA4 data cannot be obtained without writing sensitive packet payloads, record the available metadata and mark the missing item rather than weakening redaction.

### 2.5 Capture cleanup

Raw temporary files remain under `/tmp`. After producing a sanitized research report, stop the proxy and delete raw flows, packet captures, temporary CA/private keys and copied configuration. Do not add credentials or raw captures to the repository.

## 3. Comparison model

Compare official 2.1.258 against these cc-bridge surfaces:

- `src/model/identity.rs`: release metadata and full env shape
- `src/service/rewriter.rs`: message headers, beta selection and body identity rewriting
- `src/service/oauth.rs`: token test and usage requests
- `src/service/telemetry.rs`: event/metrics UA, env and resource attributes
- `src/service/account.rs`: client detection tests
- `src/tlsfp/tlsfp.rs` and `craftls/`: only if transport evidence differs

Classify every observed field as:

1. release-bound constant;
2. platform/account identity field;
3. request/session dynamic field;
4. feature/model conditional field;
5. stable protocol field;
6. unrelated/no-change.

## 4. Release profile ownership

Avoid independent version literals across services. The recommended shape is to keep the authoritative Claude Code release metadata with canonical identity code in `src/model/identity.rs`:

- Claude Code version
- version base
- build time
- Stainless/Anthropic JS SDK package version when confirmed by capture

Services format their own endpoint-specific UA and headers from these constants. Beta-selection rules remain owned by `src/service/rewriter.rs` because they are model/request protocol behavior rather than account identity.

## 5. Existing-account compatibility

Confirmed behavior:

- Newly created upstream accounts managed by cc-bridge persist 2.1.258 release metadata.
- Accounts loaded with old or missing release metadata are normalized in memory to the current release profile before any request, telemetry or admin response uses them.
- Create/update paths persist normalized metadata, so old rows converge naturally when edited without requiring a cross-database JSON migration.
- Preserve per-account platform, architecture, Node version, terminal, package managers, device ID and process ranges.

This avoids SQLite/PostgreSQL JSON migration complexity while ensuring existing accounts do not continue sending `2.1.81` at runtime. It also avoids rewriting unrelated account fingerprint fields.

## 6. Beta and auxiliary request policy

Do not add every beta string found in the binary. Implement only the runtime subset and conditions established by capture/static control-flow analysis. Preserve model-specific behavior, including Haiku exclusions and `[1m]` handling.

Usage, token-test and telemetry endpoints must use the endpoint-specific header set observed from 2.1.258; they must not blindly reuse the messages header set.

## 7. TLS decision gate

- If the official client transport profile is equivalent to the current `NODEJS_FINGERPRINT` within captured dimensions, leave `src/tlsfp/` and `craftls/` unchanged and document the evidence.
- If it differs, pause implementation planning, load the craftls TLS spec, document the exact delta and add a narrowly scoped transport change plus local regression tests.

## 8. Risk and rollback

- A consistent client profile removes obvious contradictions but cannot guarantee account safety or bypass platform enforcement. A relay-only capture also cannot prove every direct Claude.ai OAuth auxiliary behavior.
- The principal implementation risk is claiming a new version while retaining stale SDK/beta/env/transport characteristics.
- Rollback is a single release-profile revert plus any evidence-backed beta/TLS delta. No schema rollback should be required under the recommended in-memory normalization design.

## 9. Capture conclusions incorporated into implementation

The capture is complete. The implementation baseline is now:

- release `2.1.258`, build `2026-09-01T21:54:40Z`
- Stainless package `0.112.1`
- darwin Stainless OS `MacOS`
- runtime `node` / `v26.3.0`
- preserve UA suffix/entrypoint while replacing only the version
- body-aware beta rules including thinking token count, cache diagnosis, mid-conversation system and effort
- Haiku can include `claude-code-20250219`; do not retain the old blanket exclusion
- ensure first-party `x-client-request-id` for both generic API and incoming Claude Code paths
- stop forcing redact-thinking without a current condition
- use `CanonicalEnvData.is_running_with_bun` rather than serializing a hard-coded false value
- no craftls change under the available transport evidence

The sanitized evidence is recorded in `research/client-2.1.258-capture.md`; all `/tmp` capture artifacts were deleted after the report was written.

## 10. Sanitized production fingerprint audit

### 10.1 Scope

The audit is part of this task because it validates the same release/header/telemetry invariants being changed. It is not a general request logger and does not justify a separate task tree.

Enable with:

```env
FINGERPRINT_AUDIT_ENABLED=true
```

Default is false. The output path is `<LOG_DIR>/fingerprint-audit.jsonl`.

### 10.2 Event model

Write three bounded event classes:

1. `profile_snapshot`: startup and effective account profile changes.
2. `hourly_summary`: per-account counters for traffic, client entrypoints/models/features, telemetry delivery and mismatch totals.
3. `fingerprint_anomaly`: immediate record when a known invariant is violated.

Every record includes `schema_version`, RFC3339 UTC timestamp, numeric account ID and event type. Do not write one full record per successful request; requests update in-memory counters.

### 10.3 Allowlist

Allowed values:

- effective Claude/build/Stainless/runtime/platform/arch profile
- auth type and `auto_telemetry` boolean
- normalized model family/model ID
- normalized UA entrypoint and client type
- ordered beta names
- thinking/effort/stream booleans or enums
- status class/count, latency bucket and telemetry endpoint/event-name counts
- presence booleans for session/request/account/org identifiers
- mismatch names and counts
- internal dropped-audit counter

Forbidden values:

- Authorization, API/setup/access/refresh tokens, cookies
- email, prompt/system prompt, response or tool content
- account/org/device/session/request ID values
- relay/proxy URL, DSN, proxy credentials or raw headers/body

The writer accepts typed sanitized observations, not arbitrary JSON or raw request objects.

### 10.4 Aggregation and anomalies

Maintain per-account counters in the audit worker. Flush an hourly summary and reset interval counters. Emit immediate anomaly records for:

- effective version/build/Stainless mismatch
- incoming UA suffix/entrypoint lost after rewrite
- Stainless OS/runtime mismatch
- required beta missing, stale forced beta or duplicate beta
- request thinking/effort inconsistent with generated telemetry
- first-party outbound request missing session/request ID
- auto telemetry send failure or event/profile version mismatch

Use actual usage/telemetry counters where already available; do not add fabricated token/cost values to the audit.

### 10.5 IO behavior and retention

Use a bounded `tokio::mpsc` channel and `try_send` from request paths. Queue full increments an atomic dropped counter and returns immediately. A background task owns a dedicated `rolling_file` appender:

- active file `fingerprint-audit.jsonl`
- 10 MiB per file
- 6 history files plus active
- Unix permission `0600` after creation/rotation where supported

Serialization or IO failure is logged without failing the request. The writer flushes on its interval and normal shutdown where possible; no hard guarantee is placed on the last buffered audit records during process crash.

### 10.6 Integration points

- `src/config.rs`: enable flag; path derives from existing `LOG_DIR`.
- new narrowly scoped audit module under `src/service/` or `src/logging/` owning schema, channel, aggregation and writer.
- `src/main.rs`: construct/start the audit service and retain its guard/task handle.
- `src/service/gateway.rs`: submit sanitized incoming/effective/outbound header/body-feature observations and response status counters.
- `src/service/telemetry.rs`: submit telemetry session/send/event-name/profile observations.

Do not expose a public/admin endpoint in this task; the user can provide the rotated NDJSON file for offline review.
