# 网关与业务协议约束

这些规则是项目特有行为，不是通用反向代理默认值。改动前应先阅读 `src/service/gateway.rs`、`account.rs`、`limit.rs`、`rewriter.rs`、`telemetry.rs` 及其相邻测试。

## 鉴权与账号模式

- `/admin/*` 接受 `x-api-key` 或 `Authorization: Bearer` 中的管理员密码，并使用 constant-time compare，见 `src/middleware/auth.rs::admin_auth`。
- 网关 fallback 只接受数据库中 status 为 active 的 API token。token 的 `allowed_accounts` 和 `blocked_accounts` 参与账号过滤，见 `ApiToken` helpers 与 `AccountService::select_account`。
- Setup Token 创建/更新必须有 `setup_token`，并清空 OAuth 字段。
- OAuth 必须有 `refresh_token`；access token 可为空并在首次使用时刷新。两种模式最终通过 `resolve_upstream_token_with` 汇合。
- OAuth refresh 使用账号级 lock、5 分钟提前刷新窗口和等待重读，防止多请求重复刷新。

## 调度与粘性

选择顺序固定为：有效 sticky binding -> 可调度账号过滤 -> 最低 priority 组 -> 组内随机。sticky TTL 为 24 小时。Claude Code 使用 `metadata.user_id` 中 session id；普通 API 用 UA 与 system/首条消息哈希。参考 `generate_session_hash` 的回归测试。

并发 slot 获取失败返回 429。slot 在流式 response body 被完全消费或 drop 时释放；不要在 `forward_request` 返回 headers 时释放，`SlotHeldStream` 测试覆盖中途断开与 pending stream。

## 上游转发

- 上游固定为 `https://api.anthropic.com`，query 中保证有 `beta=true`。
- 所有调用通过 `src/tlsfp/tlsfp.rs::make_request_client`，以保留 Node.js TLS fingerprint、代理和 idle read timeout。
- 账号真实 token 只在最终 outbound headers 中注入。
- 响应过滤 AI gateway 指纹前缀：`x-litellm-`、`helicone-`、`x-portkey-`、`cf-aig-`、`x-kong-`、`x-bt-`。

## 状态码策略

| 上游结果 | 本地行为 |
| --- | --- |
| send 失败 | 返回 502 `upstream request failed`，尚未收到上游 HTTP response |
| 403 | 原响应继续返回，同时把账号本地标为 disabled，原因 `403 认证失败` |
| 429 | 不切账号、不 retry；保留 429 与可用 headers，body 换成通用 `rate_limit_error` |
| 任意 5xx | 不 retry；保留原状态，body 换成通用 `api_error`，移除跟踪/基础设施 headers |
| 其它状态 | 保留 status/body，仍经过 gateway fingerprint header 过滤 |

429 不换号是为了保留 prompt cache，属于明确业务选择。不要引入“遇到错误自动换账号”的通用 retry。

## 限流状态机

`LimitStore` 是 selector 的内存热态，DB `usage_data` 是异步快照：

- OAuth 解析 `anthropic-ratelimit-unified-*`；5h/7d utilization 达到 97% 或 status rejected 时不可调度。
- Setup Token 解析 requests/tokens/input/output RPM/TPM；任一 remaining/limit 低于 3% 且未 reset 即预抢。
- 无 quota headers 的 429 使用 `retry-after`，缺失时隔离 60 秒。
- 首次填充、5 分钟 TTL、状态变化、阈值跨越、429 和 RPM/TPM 新预抢会触发异步 DB flush。
- `representative_claim=seven_day_sonnet` 写入 Sonnet overlay；不能把全账号 status 标为 rejected，Opus 仍应可调度。
- 手工 `/api/oauth/usage` 只支持 OAuth，成功后同时更新 DB 与内存；60 秒 freshness cache 和 429 cooldown 必须保留。

修改这一状态机时应优先扩展 `compute_new_state`、`judge_availability`、header parsing 的纯函数测试，不要只测最终 UI。

## 请求与遥测重写

`Rewriter` 按路径分派：

- `/v1/messages`：模型 `[1m]`、Claude Code metadata/system prompt、API 注入模式、billing/CCH、空 text/cache_control 等。
- `/api/event_logging/batch`：设备、账号、环境、process 和 user attributes。
- `/api/eval/*`：GrowthBook identity 字段并移除 gateway host。
- 其它 JSON：仅在字段存在时改写通用 identity。

非 JSON body 原样返回。API 模式会删除内部 `_session_id` 后才发上游。beta header 由 `compute_betas_for_model` 集中计算，修改模型规则必须更新其单测。

自动遥测路径被本地拦截并返回假成功，真实代发由后台会话执行。`send_telemetry` 的失败只 `warn!`，不能让主请求失败；Datadog 客户端直连不在本项目拦截范围，见 `README.md`。

## Scenario: 动态模型级周额度

### 1. Scope / Trigger

- `/api/oauth/usage` 出现 `limits[].type=weekly_scoped`，或实时响应头出现模型级 7 天窗口时使用。

### 2. Signatures

- OAuth JSON：`limits[].{type,scope.model.model_group,scope.model.display_name,utilization,resets_at}`。
- 实时 header bucket：`7d_sonnet`、`7d_oi`。
- representative claims：`seven_day_sonnet`、`seven_day_overage_included`。

### 3. Contracts

- OAuth utilization 是 0..100；`LimitState` 内部统一存 0..1；写回 `usage_data` 时恢复 0..100。
- `7d_sonnet` 规范化为 group `sonnet`，`7d_oi` 规范化为 group `fable`。
- Sonnet/Fable scoped rejected、97% 阈值和短期 429 只影响对应模型。
- 旧 `seven_day_sonnet` 仍作为输入/输出 mirror；动态 `limits` 是新前端的主要来源。
- 未知 group 可以存储和展示，但没有请求模型映射前不得参与 selector。

### 4. Validation & Error Matrix

- `limits: null`/`[]` -> 清空旧模型级窗口且不造 0% 项；旧响应完全缺失 `limits` -> 保留实时 header 热态。
- 非 `weekly_scoped`、空 group、非法 utilization/reset -> 忽略该项。
- scoped claim + rejected -> 不写账号级 `status=rejected`。
- 新式显式 scoped header + 通用 7d -> scoped 与账号级分别保存；旧式只有 scoped claim + 通用 7d -> 通用 7d 路由到 scoped overlay。

### 5. Good/Base/Bad Cases

- Good：Fable 达到额度只使 Fable 请求换账号，Sonnet/Opus 仍可用。
- Base：旧响应只有 `seven_day_sonnet`，页面仍显示 Sonnet。
- Bad：把所有非 Sonnet 模型都当 Opus，导致 Fable 429 污染整个账号。

### 6. Tests Required

- modern/legacy/null/malformed OAuth fixtures。
- `7d_oi` 与 `7d_sonnet` header parsing。
- Fable/Sonnet/Opus availability 交叉矩阵。
- `build_usage_json` 后动态 limits 仍存在且排序稳定。

### 7. Wrong vs Correct

Wrong：根据请求成本 ledger 反推 quota，或缺少 scoped 数据时显示固定 0%。

Correct：只使用 OAuth usage/实时 rate-limit 数据，并按实际存在的 model group 判定和展示。

## Scenario: Claude Code release profile alignment

### 1. Scope / Trigger

- Trigger: changing the emulated Claude Code release, request headers, beta selection, canonical env or automatic telemetry profile.
- Current verified darwin arm64 baseline: Claude Code `2.1.258`, build `2026-09-01T21:54:40Z`, Stainless `0.112.1`, runtime `node/v26.3.0`, Stainless OS `MacOS`.

### 2. Signatures

- Authoritative constants: `src/model/identity.rs::{CLAUDE_CODE_VERSION, CLAUDE_CODE_BUILD_TIME, CLAUDE_CODE_STAINLESS_VERSION, CLAUDE_CODE_RUNTIME_VERSION}`.
- Runtime normalization: `normalize_canonical_env_json(&mut Value)` and `parse_canonical_env(&Value)`.
- Request-aware beta selection: `compute_betas_for_request(model_id, body)`.
- Existing-account persistence: account rows normalize on read; `AccountStore::update` persists normalized `canonical_env` when the account is edited.

### 3. Contracts

- Incoming Claude Code UA keeps its suffix/entrypoint and only replaces the release token: `sdk-cli`, `cli` and Agent SDK metadata must not collapse to one value.
- First-party outbound requests always have normalized Stainless version/OS/runtime plus `X-Claude-Code-Session-Id` and `x-client-request-id`.
- Incoming beta order is preserved; missing bridge-required auth/first-party beta values append without duplicates.
- Beta selection uses both model and body: thinking adds thinking-token-count; effort can add mid-conversation/effort; Claude 4.5 Haiku keeps `claude-code-20250219`; legacy Claude 3 Haiku does not.
- `redact-thinking-2026-02-12` is not added by default without a current request condition or incoming value.
- Existing account platform, arch, terminal, package managers, device ID and process ranges survive release normalization.

### 4. Validation & Error Matrix

- Missing/partial `canonical_env` -> fill release-bound defaults, retain unknown JSON keys, do not fail account load.
- Unknown incoming beta -> retain after known required beta values; invalid audit values are normalized to `other`.
- Missing first-party request/session ID -> generate before forwarding and report an audit anomaly if still absent.
- No direct ClientHello/JA3/JA4 evidence -> do not modify craftls or the product TLS fingerprint.

### 5. Good/Base/Bad Cases

- Good: `claude-cli/2.1.81 (external, sdk-cli)` becomes `claude-cli/2.1.258 (external, sdk-cli)` with Stainless `0.112.1` and body-aware beta values.
- Base: a generic API request receives the canonical CLI header set and generated request/session IDs.
- Bad: updating only the UA version while leaving `canonical_env=2.1.81`, Stainless `0.70.0`, or forcing every entrypoint to `cli`.

### 6. Tests Required

- Release normalization preserves unknown/non-release fields and persists on account edit.
- Header tests cover UA suffix preservation, Stainless values and generated first-party IDs.
- Beta tests cover Sonnet thinking+effort, Claude 4.5 Haiku order, legacy Claude 3 and `[1m]`.
- Telemetry tests assert request entrypoint/thinking/effort and env/metrics release consistency.

### 7. Wrong vs Correct

Wrong: independently hard-code a version or Stainless value in `oauth.rs`, `rewriter.rs` or `telemetry.rs`.

Correct: use the identity-owned release constants and endpoint-specific formatting/selection logic.
