# 调研证据：请求性能监控

日期：2026-09-24。本次为源码调查，没有执行生产请求、读取生产数据库或确定当天慢响应的根因。

## 基线

- 工作区 `/Users/qiubingnan/RustProject/cc-bridge`，分支 `ccb`，HEAD `b2a0c21`。原有未跟踪 Trellis/平台配置和 fingerprint-audit.jsonl 保留。
- 参考 `/Users/qiubingnan/GoProject/sub2api`，HEAD `dbc8ae658cfc1c012160752582925e45115e2f3a`，读取时工作区干净；未联网确认其是否最新。
- 用户原话：今天响应特别慢，经常三十几分钟才能回复，询问性能指标和首字响应，要求参考 sub2api。调研后用户授权“可以，进任务吧”。

## sub2api 可借鉴内容

| 证据（相对 sub2api 根目录） | 当前行为 | 本任务决策 |
| --- | --- | --- |
| `backend/internal/service/gateway_service.go:8599` | usage log 保存 duration/first_token、模型、账号、API Key 和 token | 请求明细可关联相同维度；性能不依赖 usage 是否存在 |
| `backend/internal/service/gateway_service.go:4317` | Forward 才开始该耗時計时 | 我们统一从 fallback 进入、鉴权前计时，并保留网关阶段 |
| `backend/internal/service/gateway_service.go:7463` | 首个非空非 DONE SSE data 触发 FirstTokenMs | 不能原样照搬首字口径，必须区分 heartbeat/metadata/content/text |
| `backend/internal/repository/ops_repo_dashboard.go:797` | 原始 usage 计算 P50/P90/P95/P99/avg/max | 首版采用 P50/P95/P99/max/count，口径明确 |
| `backend/internal/repository/ops_repo_dashboard.go:242` | preagg 跨段 percentile 是近似组合 | 不复制预聚合分位数平均/最大合并作为精确分位数 |
| `backend/internal/repository/ops_repo_dashboard.go:776`、`:900` | TPS 包含 input/output/cache 等总 token / 时间窗口 | 单请求生成速度另算，不同总吞吐混用 |
| `backend/internal/service/ops_request_details.go:43` | 可过滤慢请求并按 duration_desc 排序 | 实现慢请求列表及详情 |
| `backend/internal/service/ops_realtime_models.go:27` | 实时账号并发容量、占用、排队 | 我们优先提供具体进行中请求年龄/阶段，而不是只有并发计数 |
| `backend/internal/service/ops_upstream_context.go:24` | auth/routing/upstream/response 的可选阶段耗时 | 增加网关自身可观察的阶段，不能推断上游内部排队 |
| `backend/internal/repository/ops_repo_latency_histogram_buckets.go:13` | 最大延迟档为 2000ms+ | 增加到 30 分钟以上的分桶 |
| `backend/internal/service/ops_alert_evaluator_service.go:233` | 持续越界/冷却时间控制告警 | 可供后续告警设计参考，本期不做通知/自动处置 |
| `frontend/src/components/admin/usage/UsageTable.vue:163` | 首字及耗时明细展示，缺失值为 — | 保留 null 表达，扩充内容/正文含义 |

## cc-bridge 接入依据

| 证据（相对当前仓库） | 发现 |
| --- | --- |
| `src/handler/router.rs:121` | gateway_fallback 负责真实鉴权；必须在这里开始才覆盖 auth 和本地拒绝 |
| `src/service/gateway.rs:147`、`:178`、`:222`、`:252`、`:298`、`:354` | 已有 perf 阶段可定位 body/调度/slot/改写/token |
| `src/service/gateway.rs:386`、`:540` | PERF_TRACE total 在 Response 返回前；body 后续才被消费 |
| `src/service/gateway.rs:319`、`:328` | UsageAttempt 开始晚于 token resolve，不能作网关完整耗时起点 |
| `src/service/gateway.rs:457`、`:474` | send 错误提前返回，所谓 upstream_send_ttfb 实际停在收到响应头 |
| `src/service/gateway.rs:542` | 429/5xx 交由后台 drain，不能把 drain 耗时记为下游等待 |
| `src/service/usage.rs:448` | CompletedInferenceObservation 出口受 telemetry_enabled 控制 |
| `src/service/usage.rs:519`、`:545` | 现有 stream wrapper 已覆盖 bytes/error/EOF/drop，但错误信息没有作为独立性能终态持久化 |
| `src/service/usage.rs:576` | first_byte_at 在解压/SSE 解析前记录，ttft_ms 并非有效内容时间 |
| `src/service/usage.rs:584`、`:597` | parser complete/invalid 可提前消费 attempt，性能生命周期必须独立 |
| `src/service/usage.rs:1079` | 现有 SSE 处理 message_start/message_delta/message_stop，可扩展内容进展观察 |
| `src/model/usage.rs:69`、`:102` | 持久事件/聚合主要为 token 和成本，不保存性能 |
| `src/tlsfp/tlsfp.rs:324` | 300 秒 read idle timeout；持续心跳允许超过 30 分钟，尚不能证明当日慢请求原因 |
| `src/store/db.rs:11`、`:50`、`:235` | schema v3；内建双数据库迁移末尾 stamp |
| `src/main.rs:87` | 有界内部 completion 管道和 composition root，可沿用非阻塞采集模式 |
| `README.md:237` | 支持多实例 + Redis，但默认单实例；首版 active 必须明确当前实例范围 |

## 规范与相邻任务

- 已读取 gateway/backend：index、usage-analytics、persistence-and-cache、error-and-logging、testing。
- 已读取 web/frontend：index、api-contracts、architecture-and-style、testing；guides/index、project-commands。
- 相邻 `.trellis/tasks/08-12-usage-analytics/design.md` 已有 batch writer、原样透传、无外键历史记录、双数据库和计费边界，复用其契约而不修改该任务。
- 指纹 audit 的 in_flight 仅截至响应头（error-and-logging spec），不能承担本任务的完整流式活跃请求监控。

## 已知观测限制

模型内部 queue/prefill、客户端点击到请求进入、客户端缓冲/渲染和工具执行不在网关观测范围。SSE 持续思考可导致首正文很晚，但不等于无进展。客户端/服务端取消有时只能识别 future/body dropped。进程崩溃与有界队列丢弃导致的缺口需要诚实显示。
