# 请求性能监控契约

## 1. Scope / Trigger

修改 `/v1/messages` 全生命周期性能采集、`/admin/performance*`、进行中快照或 schema v4 性能历史时使用。不同于 usage ledger（仅可信用量）及 audit in_flight（仅到响应头）。

## 2. Signatures

- `PerformanceService::begin() -> Option<PerformanceGuard>` 在 gateway_fallback 鉴权前调用。
- `PerformanceGuard::response(Response) -> Response` 移交到 HTTP Body；原 frames/trailers/size_hint 透传。
- `UsageService::observe_stream_with_performance(stream, attempt, Option<PerformanceHandle>)` 复用已有解压和 SSE/JSON parser。
- `PERFORMANCE_MONITORING_ENABLED` 默认 true；关闭不影响查询已有历史。
- `request_performance_events`：内部 request_id PK、epoch-ms/time/dimension/outcome/metric 投影列 + typed event_json 快照；无级联外键。
- `GET /admin/performance`、`/active`、`/requests`、`/requests/:request_id`、`/dimensions`；全部 admin_auth。

## 3. Contracts

- started_at 早于鉴权；duration 直到 body EOF/error/drop；message_stop 只记录模型结束，不能提前结束活跃请求。
- first_byte 是上游原始 body；first_content 为非空 text/thinking/tool-input，first_text 仅正文。heartbeat、message_start、signature、空内容均不推进内容时间。
- 非流式 JSON 只能证明完成，首内容/首正文和生成速度为空；一次批量返回也不估算速度。
- HTTP 200 不等于 success；流错误/缺少终止/取消/解析未知要单列。无法确定取消责任时只称 aborted。
- 自动遥测开关和 usage early-finalize 不影响性能生命周期；429/5xx 后台 drain 不计入下游耗时。
- 主路径仅同步短内存更新与 try_send；4096 completion 队列、128 条/200ms 批写、active 上限 10000，缺口必须计数。
- 当前实例 active；共享数据库历史。保留开始时间所在的新加坡自然日及之前共 30 日；不删除计费用量。
- 时间过滤 `start_at/end_at` 为 RFC3339，范围最多保留窗口；page_size 1..100。时延指标对象 `sample_count/p50/p95/p99/max` 为 nearest-rank，按非空成功样本分别计数，不能平均分位数。
- 不保存正文、prompt、凭证或任意外部错误文本；model/request-id/stop-reason 用有界安全字符规范化。
- SQLx Any 0.7 将 NULL 丢失原类型并转换为 INT4。可选值使用固定类型 sentinel + SQL NULLIF；不能假定 SQL CAST 会修复缓存的绑定类型。
- QueryBuilder<Any> 输出 `?`，PG 不支持。性能查询编号化仅来自 push_bind 的占位符；所有用户值保留参数绑定，不能拼接用户值或重写其内容。

## 4. Validation & Error Matrix

| 输入/事件 | 结果 |
| --- | --- |
| 心跳持续 30 分钟 | active age/idle 增长，first_content=null |
| thinking -> text | 首内容早于首正文，thinking 是内容进展 |
| tool-only + message_stop | success、first_text=null |
| SSE error + HTTP200 | stream_error |
| EOF 无 message_stop | incomplete |
| 解析不支持/失败 | 原字节透传，终态 unknown（若已有明确错误优先），健康计数增加 |
| future/body drop | aborted 或已经确认的错误；终态只写一次 |
| completion 队列满/DB 失败 | 主响应正常、dropped/write_failed 增加 |
| 错误枚举/反向时间/超保留范围/非法分页 | 400 |
| 缺少管理员认证 | 401 |

## 5. Good / Base / Bad Cases

- Good：压缩 SSE 任意 chunk 到达，原字节完全一致，心跳不误算首字，EOF 才写一次性能记录。
- Base：鉴权失败无账号/Token 维度仍有 local_error；关闭自动遥测仍采集。
- Bad：从 HTTP headers 的“total”推断完整响应，或把账单记录数当所有推理请求数。

## 6. Tests Required

- guard 持有、EOF/drop/abort、typed timeout/error、queue full 和 thirty-minute heartbeat；一次性终态和无敏感正文。
- identity/gzip/deflate/br/zstd 通过同一 parser 的性能事件，验证字节相等、early-finalize 不结束性能请求。
- SQLite fresh/v3 migration、幂等、null/非空交替记录、分页/筛选/nearest-rank、维度、retention。
- `PERFORMANCE_TEST_POSTGRES_DSN=<isolated test db> cargo test --lib postgres_performance_queries_match_sqlite -- --ignored`，仅隔离测试库，测试使用私人 schema。
- `cargo test --lib`、`cargo check`、前端 strict build；本地绑定测试需要允许 loopback 端口。

## 7. Wrong vs Correct

Wrong：`forward_request().await` 返回后结束计时；对每个 chunk 写库；把 Any 的可选值直接混用 null/i64 后认为 SQLite 通过即可。

Correct：唯一 guard 移交给响应 Body，原 parser 提供进度通知，终态 try_send，分别执行 SQLite 与独立 PG 的真实批量回归。
