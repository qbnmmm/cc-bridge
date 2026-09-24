# Claude Code 2.1.258 Capture Report

Date: 2026-09-02

## Safety controls

- No local Claude.ai login was used.
- No OAuth credential was extracted from cc-bridge or the local machine.
- The only real requests used the existing relay configuration already present in the active CAC settings.
- Claude Code ran with safe/restricted mode, no tools, no MCP, no project settings, no updater, no telemetry and no nonessential traffic.
- The system prompt and user prompt were reduced to a fixed `OK` response test.
- The capture addon redacted authorization, host, identifiers, prompts and response content before writing JSONL. It did not write mitmproxy flow files.
- Redaction was verified first with synthetic HTTP and HTTPS secrets.
- A separate first-party-host experiment used a fake token and a blocking loopback proxy; the proxy returned a local synthetic 400 response and did not connect to Anthropic.

## Account-impact summary

Two serial requests reached the existing relay:

| Scenario | Result | Input tokens | Output tokens |
| --- | --- | ---: | ---: |
| Sonnet | success, exact `OK` | 185 | 4 |
| Haiku | success, exact `OK` | 165 | 44 |
| Total | 2 successful requests | 350 | 48 |

No tools, retries, concurrent sessions, 1M context, fast mode or auxiliary telemetry requests were used. The Sonnet request metadata was not persisted because the first version of the capture addon failed to JSON-encode ALPN bytes; the request itself succeeded normally. The addon was fixed and revalidated before the Haiku request. Sonnet request generation was also captured against a local synthetic endpoint using the same 2.1.258 binary and relay-token authentication mode.

## Authoritative release metadata

From the exact binary `/Users/qiubingnan/.cac/versions/2.1.258/claude`:

- Claude Code version: `2.1.258`
- Build time: `2026-09-01T21:54:40Z`
- Git SHA: `b3cd543a1f6fcdf4d8fabc0f5e5538d2ee7f38e1`
- Stainless/Anthropic JS SDK package version: `0.112.1`
- Native darwin arm64 runtime exposed through Stainless: `node` / `v26.3.0`
- Stainless OS value: `MacOS`
- Stainless arch value: `arm64`

This directly conflicts with cc-bridge's current `2.1.81`, build time `2026-03-20T21:26:18Z`, Stainless `0.70.0`, `Mac OS X`, and darwin Node presets `v22.15.0` / `v24.3.0`.

## Client-to-relay request profile

The fully captured real-relay Haiku request used:

- Method/path: `POST /v1/messages?beta=true`
- HTTP version negotiated with the relay: HTTP/1.1
- User-Agent: `claude-cli/2.1.258 (external, sdk-cli)`
- `X-Stainless-Lang: js`
- `X-Stainless-Package-Version: 0.112.1`
- `X-Stainless-OS: MacOS`
- `X-Stainless-Arch: arm64`
- `X-Stainless-Runtime: node`
- `X-Stainless-Runtime-Version: v26.3.0`
- `X-Stainless-Retry-Count: 0`
- `X-Stainless-Timeout: 600`
- `X-Claude-Code-Session-Id`: present
- `x-client-request-id`: absent for the configured third-party relay host
- `anthropic-dangerous-direct-browser-access: true`
- `anthropic-version: 2023-06-01`
- `x-app: cli`
- Client-side negotiated TLS cipher: `TLS_AES_256_GCM_SHA384`

The `sdk-cli` UA suffix is produced by `--print`. Static binary control flow shows normal interactive CLI defaults to `cli`. Therefore cc-bridge must preserve the incoming entrypoint suffix when rewriting the version instead of forcing every Claude Code request to `(external, cli)`.

### Real-relay Haiku beta order

```text
interleaved-thinking-2025-05-14
thinking-token-count-2026-05-13
context-management-2025-06-27
prompt-caching-scope-2026-01-05
claude-code-20250219
```

The request body resolved `haiku` to `claude-haiku-4-5-20251001` and used enabled thinking. This disproves the current cc-bridge rule that Haiku must never advertise `claude-code-20250219`.

## First-party-host generation without Anthropic traffic

To inspect host-dependent headers safely, the official client was configured with `https://api.anthropic.com` plus a fake token and routed to a blocking local proxy. The proxy generated the TLS certificate, captured the request, returned a synthetic 400 and used `connection_strategy=lazy`; it did not forward upstream.

### Sonnet

Resolved model: `claude-sonnet-5`

```text
claude-code-20250219
interleaved-thinking-2025-05-14
thinking-token-count-2026-05-13
context-management-2025-06-27
prompt-caching-scope-2026-01-05
mid-conversation-system-2026-04-07
effort-2025-11-24
cache-diagnosis-2026-04-07
```

The request included both `X-Claude-Code-Session-Id` and `x-client-request-id`.

### Haiku

Resolved model: `claude-haiku-4-5-20251001`

```text
interleaved-thinking-2025-05-14
thinking-token-count-2026-05-13
context-management-2025-06-27
prompt-caching-scope-2026-01-05
claude-code-20250219
cache-diagnosis-2026-04-07
```

The request included both `X-Claude-Code-Session-Id` and `x-client-request-id`.

`oauth-2025-04-20` was absent because the synthetic process had an environment token, not a real Claude.ai subscriber credential. cc-bridge's upstream-account auth mode still needs the OAuth beta where required, but the remaining first-party beta ordering can be derived from this experiment.

## Header order observed for first-party-host requests

```text
Accept
Authorization
Content-Type
User-Agent
X-Claude-Code-Session-Id
X-Stainless-Arch
X-Stainless-Lang
X-Stainless-OS
X-Stainless-Package-Version
X-Stainless-Retry-Count
X-Stainless-Runtime
X-Stainless-Runtime-Version
X-Stainless-Timeout
anthropic-beta
anthropic-dangerous-direct-browser-access
anthropic-version
x-app
x-client-request-id
Connection
Host
Accept-Encoding
Content-Length
```

This is recorded as evidence, not automatically an implementation requirement. The current Rust client and HTTP protocol negotiation may own final wire ordering.

## Confirmed cc-bridge deltas

| Surface | Current cc-bridge | 2.1.258 evidence | Direction |
| --- | --- | --- | --- |
| Version | `2.1.81` | `2.1.258` | update centrally |
| Build time | `2026-03-20T21:26:18Z` | `2026-09-01T21:54:40Z` | update centrally |
| Stainless SDK | `0.70.0` | `0.112.1` | update centrally |
| Stainless OS | `Mac OS X` | `MacOS` | update mapping |
| Runtime version | `v22.15.0` / `v24.3.0` | `v26.3.0` | update 2.1.258 darwin profile |
| UA entrypoint | always rewritten to `cli` | `sdk-cli` for print; `cli` interactive | preserve incoming suffix |
| Haiku Claude Code beta | omitted | present, later in order | update rule/order |
| thinking-token-count | omitted | present when thinking is sent | add request-body condition |
| cache-diagnosis | omitted | present for first-party host | add first-party rule |
| mid-conversation-system | omitted | present on captured Sonnet request | add request/model/body condition |
| effort | omitted | present when `output_config.effort` is sent | add body condition |
| x-client-request-id | API mode only; not ensured in CC mode | required/generated for first-party host | ensure for upstream first-party request |
| redact-thinking | forced for most models | absent from all 2.1.258 captures | stop forcing without a current condition/evidence |
| TLS profile | `NODEJS_FINGERPRINT` | only negotiated cipher/ALPN observed | insufficient evidence for craftls change |

## Recommended rewrite strategy

1. For incoming Claude Code traffic, preserve the incoming UA entrypoint and version-independent suffix; replace only the version token with the account's normalized release version.
2. Preserve the incoming 2.1.258 beta order, then add only upstream-required auth/first-party betas that are absent. Do not prepend a stale recomputed list that destroys the official order.
3. Ensure `x-client-request-id` exists before forwarding to first-party Anthropic, because the official first-party-host request generated it even when the relay-host request did not.
4. For generic API injection, compute the 2.1.258 beta set from model plus body features (`thinking`, `output_config.effort`, mid-conversation behavior), not model name alone.
5. Remove the unconditional/default `redact-thinking` addition unless later static control-flow evidence identifies a current 2.1.258 condition.
6. Leave TLS/craftls unchanged in this task unless a later direct, sanitized transport trace proves a concrete ClientHello/HTTP2 mismatch.

## Evidence limitations

- The real relay negotiated HTTP/1.1; this does not prove that the 2.1.258 client cannot use HTTP/2 against another server.
- Only the negotiated cipher was recorded, not a full ClientHello extension/cipher ordering or exact JA3/JA4.
- No real Claude.ai subscriber OAuth login was used. The OAuth beta remains an existing account-mode requirement rather than a live-capture result.
- Telemetry/event payloads were intentionally disabled to minimize account impact. Environment-shape changes rely on static binary analysis.
