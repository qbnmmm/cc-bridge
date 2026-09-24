# Claude Code 2.1.258 Telemetry Delta

Date: 2026-09-02

## Evidence boundary

Live telemetry was intentionally disabled during the relay capture to minimize account impact. No telemetry request was sent by the capture run. The findings below come from static control-flow analysis of the exact 2.1.258 darwin arm64 binary and comparison with `src/service/telemetry.rs` / `src/model/identity.rs`.

## Envelope and serialization

2.1.258 builds telemetry in three internal groups:

- `core_metadata`: model/session/user/client/beta/entrypoint/agent fields
- `user_metadata`: email and account/organization identity
- `event_metadata`: event-specific fields such as token counts, timing and cache metrics

Before sending `/api/event_logging/batch`, these are transformed back into `ClaudeCodeInternalEvent` with a flat `event_data`, plus:

- `env`
- base64 `process`
- optional `auth`
- optional base64 `additional_metadata`

The final envelope remains compatible in broad shape, but optional-field behavior and the contents of env/additional/event metadata have changed.

## Environment delta

Official 2.1.258 env includes or conditionally includes:

- `platform`, `platform_raw`, `arch`
- `node_version`
- `terminal` with `unknown` fallback
- `shell`
- `package_managers`, `runtimes`
- `is_running_with_bun`
- CI/action/remote/conductor/local-agent flags
- Claude.ai auth flag
- `version`, `version_base`, `build_time`
- deployment environment and VCS
- optional Linux/WSL/GitHub/remote/container/tags fields

Current cc-bridge differences:

- version/build/node values are 2.1.81-era.
- `build_full_env_json` hard-codes `is_running_with_bun=false` even though the model already stores the field.
- `shell` is not part of `CanonicalEnvData` and is absent from full env; it exists separately in canonical prompt env.
- many optional fields are emitted as empty strings/empty arrays. Official 2.1.258 conditionally omits most absent optional fields.

## Core metadata delta

2.1.258 core can include:

- session/model/user type/is interactive/client type
- betas
- entrypoint
- Agent SDK version
- SWE-bench identifiers
- agent/parent session/agent type/team name

Current cc-bridge always emits several absent values as empty strings and fixes `entrypoint=cli`, `client_type=cli`, `is_interactive=true`. This can conflict with `sdk-cli`, noninteractive print sessions or agent traffic.

## Additional metadata delta

2.1.258 packs optional fields into base64 JSON `additional_metadata`, including when available:

- repository hash `rh`
- head SHA
- coach/observer mode
- session kind
- has-attacher flag
- renderer mode
- subscription type
- parent agent ID
- Claude Code prompt ID

Current cc-bridge sends `additional_metadata` as an empty string in its synthetic event batch. Unknown optional values should be omitted rather than fabricated. Available stable account data such as subscription type can be included only where the official serializer does so.

## `tengu_api_query` delta

Current 2.1.258 query event supports:

- existing model/messages length/temperature/provider/build age/betas/permission mode/query source/thinking type/fast mode
- proactivity level
- message client platform
- query chain ID/depth
- effort value
- previous request ID
- other current dynamic feature metadata

Current cc-bridge lacks the newer optional fields and always reports thinking disabled, which conflicts with 2.1.258 adaptive/enabled thinking requests.

## `tengu_api_success` delta

2.1.258 retains the older token/timing/cost fields but adds many conditional values, including:

- low-priority flag and pre-normalized model
- first-attempt and invocation request identity
- effort/default-model/default-effort fields
- query chain/depth, permission and proactivity
- global cache strategy and prompt-cache TTL/reason
- text/thinking/narration/tool-use content lengths
- image/document/text input statistics
- system/tools character counts, tool count, deferred-tool count and tool-schema hash
- request-body compression details
- previous request ID and post-compaction flag
- attribution/skill data
- time since previous API call

Current cc-bridge sends a smaller fixed schema and synthesizes random token counts, timing, cost and an occasional `tengu_tool_use_success` with 40% probability. Those fabricated correlations are not grounded in the actual request/response and may be more inconsistent than omitting optional telemetry.

## Metrics delta

2.1.258 internal metrics resource includes:

- `service.name`
- `service.version`
- `os.type`
- `os.version`
- `host.arch`
- optional `wsl.version`
- aggregation temporality
- customer type (`claude_ai` or `api`)
- optional subscription type

Current cc-bridge is missing `os.version` and always sends an empty `metrics` array. Official 2.1.258 transforms real OTel metric data points when metrics are enabled; an empty periodic metric body is only a partial imitation.

## Scheduling

The default scales remain similar (roughly 10-second event export batching and 60-second metrics), but 2.1.258 uses OTel queues, server/feature configuration, sampling and opt-out checks. cc-bridge uses a deterministic background loop and generates query/success pairs plus random tool events.

## Recommended scope for this task

1. Update release-bound env/metrics values to 2.1.258.
2. Set/use `is_running_with_bun` and add the known shell value to telemetry env.
3. Preserve real entrypoint/client/interactivity when it is available; omit unknown optional core fields instead of sending empty strings.
4. Make event beta/thinking fields consistent with the actual forwarded request.
5. Stop generating random tool-use events and avoid inventing new 2.1.258 optional metadata.
6. Keep automatic telemetry default off.
7. Do not claim full 2.1.258 telemetry parity without a separately authorized live telemetry capture or a design that derives telemetry from real request/response observations.
