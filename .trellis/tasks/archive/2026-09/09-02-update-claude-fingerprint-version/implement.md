# Implementation Plan

## Phase A: evidence capture and analysis

- [x] Create a temporary redacting `mitmdump` addon under `/tmp`; do not add capture tooling to the repository.
- [x] Verify the addon output contains no Authorization, cookies, credential material, account identifiers, prompt text or response text using synthetic local input first.
- [x] Read the existing relay base URL/token from the active CAC settings directly into process environment variables without displaying or writing their values, use an empty temporary `CLAUDE_CONFIG_DIR`, start loopback capture, and launch the exact official Claude Code 2.1.258 binary. Do not perform local Claude.ai login or OAuth extraction.
- [x] Execute one minimal Sonnet `--print` request and one minimal Haiku `--print` request serially.
- [x] Record sanitized application-layer request profiles for messages and any requests naturally sent to the configured relay. Treat OAuth usage/metrics/events as static-analysis-only unless independent sanitized upstream evidence already exists.
- [x] Record available client-side TLS/ALPN/HTTP2 metadata without persisting sensitive payloads.
- [x] Stop capture, generate `research/client-2.1.258-capture.md`, verify redaction, and delete raw `/tmp` artifacts.
- [x] Build a field-by-field delta table against cc-bridge and revise `prd.md`/`design.md` if capture disproves current assumptions.

## Phase B: application profile alignment

- [x] Add authoritative 2.1.258 release constants in the canonical identity owner.
- [x] Update new account presets to use the authoritative version, version base and verified build time.
- [x] Normalize old/missing release metadata on account load and create/update while preserving all non-release account identity fields.
- [x] Replace independent `2.1.81`, stale Stainless literals, `Mac OS X` and stale runtime versions in messages, token test, usage and telemetry paths with the authoritative profile or endpoint-specific captured constants.
- [x] Replace the model-only beta builder with a request-aware builder covering thinking token count, cache diagnosis, mid-conversation system, effort, Haiku ordering and `[1m]`; preserve unknown incoming betas, retain OAuth semantics and stop forcing redact-thinking.
- [x] Update full env/event/metrics release fields and serialize the account `is_running_with_bun` value instead of a hard-coded false; retain unrelated stable fields.
- [x] Preserve incoming UA suffix/entrypoint while replacing its version, ensure first-party `x-client-request-id`, and update version-sensitive tests; keep client detection tests version-independent where the exact number is not behavior.

## Phase C: conditional transport alignment

- [x] Compare the available negotiated TLS/ALPN evidence with the current transport scope; evidence is insufficient for a full ClientHello/JA3/JA4 comparison.
- [x] Record a no-change conclusion and skip craftls edits for this task.
- [x] No direct sanitized evidence was available; transport changes remain deferred to a separate future task.

## Validation

- [x] `rg -n '2\\.1\\.81|0\\.70\\.0' src` has no unintended production literals (remaining matches are regression fixtures).
- [x] Targeted identity, account-store, rewriter, OAuth, audit and telemetry tests pass.
- [x] Task-modified files pass rustfmt; repo-wide `cargo fmt --check` was audited and only reports pre-existing drift in unchanged `src/store/token_store.rs` and `src/tlsfp/tlsfp.rs`.
- [x] `cargo test --lib` (199 passed; loopback tests run with sandbox escalation)
- [x] `cargo check`
- [x] No transport code changed; existing root TLS tests passed in the full library suite.
- [x] Reviewed diff and secret patterns; no capture artifacts or real credentials/account identifiers remain.

## Rollback points

1. Capture stage: remove only `/tmp` artifacts; repository source remains unchanged.
2. Application alignment: revert centralized release constants and normalization/beta changes together to avoid a mixed profile.
3. Transport alignment, if any: keep as a separate diff section so it can be reverted independently from application headers.

## Phase D: sanitized production audit

- [x] Add `FINGERPRINT_AUDIT_ENABLED` config with default false and derive the dedicated audit file from `LOG_DIR`.
- [x] Define a versioned, typed audit record schema with explicit allowlisted fields; do not accept raw headers, bodies or arbitrary JSON.
- [x] Implement a bounded non-blocking observation channel and background per-account hourly aggregator.
- [x] Add a dedicated 10 MiB rolling NDJSON writer with active plus 6 history files and Unix `0600` permission where supported.
- [x] Emit profile snapshots at startup/effective profile change and hourly summaries for traffic, entrypoint/model/features, telemetry delivery and mismatch counters.
- [x] Emit immediate anomaly records for version/Stainless/runtime, UA suffix, beta/thinking and first-party request-ID inconsistencies.
- [x] Integrate sanitized observations into gateway rewrite/response and telemetry session/send paths without logging token, URL, UUID or content values.
- [x] Add tests that scan serialized audit records for forbidden key names/patterns and verify queue-full/IO failure cannot fail a request.
- [x] Document how to enable, locate, rotate and safely share `fingerprint-audit.jsonl`.
