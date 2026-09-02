# 错误处理与日志

## `AppError` 是 HTTP 错误边界

`src/error.rs` 定义统一错误类型并实现 `IntoResponse`：

| Variant | HTTP |
| --- | --- |
| `NotFound` | 404 |
| `BadRequest` | 400 |
| `Unauthorized` | 401 |
| `TooManyRequests` | 429 |
| `BadGateway` | 502 |
| `ServiceUnavailable` | 503 |
| `Internal` | 500 |

SQLx `RowNotFound` 转成 `NotFound`，其它 SQLx 错误转成 `Internal`。handler/service/store 的常规失败使用 `Result<_, AppError>` 和 `?`，不要在每层重新包装成 JSON。

`AppError::Internal` 只向客户端返回 `{"error":"internal error"}`，详细内容在服务端用 `error!` 记录。可操作的输入错误可以保留消息，例如 `BadRequest("setup_token is required")`。参考 `normalize_account_auth` 和 `AppError::into_response`。

## 特殊响应不是普通 `AppError`

网关上游响应有独立协议语义：

- send 阶段失败映射成 `BadGateway("upstream request failed")`，见 `GatewayService::forward_request`。
- 上游 429 和 5xx 已拿到 HTTP 响应，必须由 `wrap_429_response` / `wrap_5xx_response` 保留状态并替换 body，不能改成普通 502。
- `gateway_fallback` 的鉴权错误用本地 `err_json` 返回稳定 `{"error": ...}`。

不要把这些路径合并成一个通用错误转换器，否则会破坏状态码、`retry-after`、body 过滤或账号状态更新。

## 外部接口错误

在 reqwest/Redis 边界用 `map_err` 加入操作上下文，参考 `refresh_oauth_token`、`fetch_usage`、`RedisStore`。上游非成功响应通常先读取 body 再构造 `AppError`，但返回给管理端前应判断是否包含凭证或不应暴露的基础设施信息。

OAuth refresh 有明确降级：如果 refresh 失败但现有 access token 尚未过期，记录 `warn!` 并继续使用；过期后才返回 `ServiceUnavailable`。不要在重构中删除这个 fallback，参考 `AccountService::refresh_oauth_access_token`。

## 日志级别

- `info!`：启动配置、缓存选择、账号筛选摘要、用量刷新结果和限流状态落盘原因。
- `warn!`：外部请求失败、403 停用、429/5xx、后台落盘失败、遥测失败和可恢复降级。
- `error!`：目前由 `AppError::Internal` 记录服务端细节。
- `debug!`：上游 URL/header、改写前后 body、遥测细节和无落盘的高频限流吸取。
- `target: "perf"`：仅 `PERF_TRACE=1` 时输出分阶段耗时，见 `gateway.rs::perf_log` 和 `benchmark/request_profile.py`。

## 敏感信息约束

账号 token、OAuth 凭证和管理员密码会以明文存在数据库；`README.md` 明确要求保护数据库访问。不要在新的 info/warn/error 日志中输出这些值。

当前 debug 路径会打印改写 body 和上游 headers；任何扩展都不得把这类内容提升到默认日志级别。修改日志代码时应优先遮蔽 `Authorization`、token、refresh token、email 和完整请求内容，而不是复制现有 debug 输出到新位置。

网关 5xx 必须继续剥离 `x-request-id`、`request-id`、`cf-ray`、`server`、`via`；对应回归测试在 `src/service/gateway.rs`。

## Scenario: 容量轮换文件日志

### 1. Scope / Trigger

- 进程日志除了 stdout 还需要进入 Docker 持久卷或宿主机目录时使用。

### 2. Signatures

- 环境变量：`LOG_DIR`，默认 `data/logs`。
- active file：`<LOG_DIR>/cc-bridge.log`。

### 3. Contracts

- stdout 与文件同时输出，并共用 `LOG_LEVEL` / `RUST_LOG` filter。
- active file 达到 30 MiB 后轮换；active + 9 个历史文件，总数最多 10。
- 文件 layer 禁用 ANSI；non-blocking writer 使用 `lossy(false)`；`tracing_appender::WorkerGuard` 必须活到 `main` 退出。

### 4. Validation & Error Matrix

- 目录可创建、文件可打开 -> 双 writer。
- 目录或 appender 初始化失败 -> stderr 明确提示，退化为 stdout-only，网关继续启动。
- 日志内容仍遵守本文件的敏感信息等级约束。

### 5. Good/Base/Bad Cases

- Good：Docker 默认写 `/app/data/logs`，由既有 data volume 持久化。
- Base：本地未设置 `LOG_DIR` 时写 `data/logs`。
- Bad：文件 writer 初始化失败后静默运行，或因日志目录不可写直接中止业务服务。

### 6. Tests Required

- 小尺寸临时 appender 触发多次轮换，断言 active + history 不超过配置数量。
- `cargo check --locked` 验证 subscriber layer 与 guard 生命周期类型。

### 7. Wrong vs Correct

Wrong：只创建 rolling appender 后立即丢弃 non-blocking guard，导致后台 writer 提前停止。

Correct：在 `main` 保存 guard，并在文件初始化失败时明确降级为 stdout-only。

## Scenario: 脱敏环境指纹审计

### 1. Scope / Trigger

- 需要线上复核 release/header/beta/telemetry 一致性，但不能保存原始请求、响应或凭证时启用。

### 2. Signatures

- 环境变量：`FINGERPRINT_AUDIT_ENABLED`，默认 `false`。
- active file：`<LOG_DIR>/fingerprint-audit.jsonl`。
- schema：versioned NDJSON；事件类型为 `profile_snapshot`、`telemetry_session`、`hourly_summary`、`fingerprint_anomaly`。

### 3. Contracts

- 只接收 typed sanitized observations，不允许把 raw header/body 或任意 `serde_json::Value` 交给 writer。
- 允许字段：数字 account ID、release/profile、归一化 model/entrypoint/client type/beta、feature enum、presence boolean、状态码和计数。
- 禁止字段：Authorization/Cookie/token/email、prompt/response/tool 内容、UUID 原值、relay/proxy URL、DSN 和代理凭证。
- request path 使用 bounded `try_send`；文件 worker 独立写入，主请求不等待磁盘。
- active file 10 MiB，active + 6 个历史文件；Unix 权限 `0600`。

### 4. Validation & Error Matrix

- 功能关闭 -> 不创建专用文件。
- queue full -> 丢弃 observation、递增 dropped counter，主请求继续。
- 目录/文件创建失败 -> `warn!`，audit 退化为空输出，网关继续。
- 任意外部字符串不符合短 token allowlist -> 写 `other`，不得原样落盘。
- serialization/flush 失败 -> `warn!`，不得转成 `AppError`。

### 5. Good/Base/Bad Cases

- Good：每小时每账号一条汇总，发现版本/UA/beta/request-ID 矛盾时立即写 anomaly。
- Base：默认关闭；部署显式开启后从 `LOG_DIR` 获取文件。
- Bad：逐请求保存完整 headers/body，或在 writer 队列满时阻塞流式响应。

### 6. Tests Required

- 禁写 key/pattern 扫描。
- 任意 model/entrypoint/feature 输入归一化为 `other`。
- queue 满时 `try_send` 不阻塞并增加 dropped counter。
- writer 输出单行合法 JSON、权限 `0600`；关闭时不创建文件。

### 7. Wrong vs Correct

Wrong：复用普通 debug request logger 生成“诊断文件”，导致 token、prompt 或 UUID 泄漏。

Correct：网关先提取 allowlisted enum/counter/presence，再提交给独立 audit worker。
