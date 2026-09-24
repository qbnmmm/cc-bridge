# 实施计划

状态：in_progress。用户已回复“开工”，方案获批且已执行 `task.py start`，本会话为 Codex inline，主会话直接实现和检查，不派发 implement/check 子代理。

## 0. 开始前

- [x] 用户授权创建任务并进入规划。
- [x] 对照本地 sub2api 与当前 gateway/usage/parser/timeout/router/schema。
- [x] 编写 PRD、design、实施清单和调查证据。
- [x] PRD 收敛阅读、需求与验收映射核对（R1–R8 / AC1–AC8）；文档与 task.json 结构检查通过。
- [x] 用户回复“开工”，明确按方案实现。
- [x] 读取 Phase 1.4 门禁，执行 `task.py start .trellis/tasks/09-24-request-performance-monitoring`。
- [x] 实现前读取 trellis-before-dev；Frontend 编码加载 vue-best-practices；质量检查加载 trellis-check。
- [x] 确认 git status、其他 usage/telemetry 活跃任务变更和最新 SCHEMA_VERSION，保留他人未提交改动。

## 1. 模型、生命周期与存储（R1/R3/R5）

- [x] 在 model/service/store 添加最小性能模块，并接入 config/main/router state；不把性能字段混进计费用量事件。
- [x] 定义明确的单调时间口径、nullable 时间字段、outcome 和 observation_quality；定义 once-only guard、active registry、bounded terminal queue 和健康计数。
- [x] 新增双数据库幂等迁移与必要索引，版本号以实施时实际最新值为准。
- [x] 实现批量幂等写入、30 日独立清理和迟到过期事件计数；队列满/写入失败不影响请求。
- [x] SQLite 测试：fresh/v3 升级、重复执行、现有 ledger 不变、插入去重、无账号/无 token 事件、过期清理。

## 2. 网关和流式采集（R1/R2/R3/R5/R6）

- [x] 在精确推理路径的 fallback 鉴权前开始计时，覆盖本地返回和 future drop；计时上下文传到 gateway。
- [x] 添加 auth/body/routing/slot/rewrite/token/send/header 阶段；不虚构上游排队或本地重试。
- [x] 复用已有压缩/SSE/JSON 解析，旁路输出有限的 content/text/thinking/tool/protocol-stop/error 观察，原始字节保持不变。
- [x] 性能生命周期独立于 usage early-finalize、解析失败和 auto_telemetry；response body EOF/error/drop 精确一次结束。
- [x] 429/5xx 下游包装返回结束时即可完成性能事件；不等待 usage 后台 drain、不重复释放 slot。
- [x] 有效生成期间速度与首正文 null 语义，非流式/不完整观测不伪造 token rate。
- [x] 确定性测试：message_start/ping 不算内容、thinking 后 text、tool-only、单块内容不可计算速度、JSON 无 usage、SSE error + HTTP200、缺终止 EOF、发送/读取 timeout、drop、guard abort、连续心跳 30 分钟虚拟时间仍活跃。
- [x] 压缩编码/任意 chunk/队列满/解析无效场景验证 bytes 和 backpressure、slot 生命周期、现有 telemetry payload 均保持契约。

## 3. 查询和管理 API（R2/R3/R4）

- [x] 实现 summary、趋势、nearest-rank 分位数和秒/分钟级直方图，按每个指标的真实非空样本数统计。
- [x] 实现 active、历史分页/排序、单请求详情、筛选维度；明确 instance scope 和观测缺口。
- [x] handler 校验范围/枚举/分页，继承 admin_auth；不暴露凭证、正文或内部堆栈。
- [x] 测试 auth、本地失败无维度、跨日开始归属、nullable 样本、成功/失败/aborted/unknown 分母、分页稳定性与危险参数拒绝。
- [x] SQLite/PG 查询同一 fixture 的分位数结果一致；PG 运行条件不足时记录实际限制，不声称已通过。

## 4. 管理台（R1/R2/R3/R4/R6）

- [x] 在 api.ts 添加 typed methods/DTO，注册 Vue `/performance` 与后端 SPA 路由，增加导航。
- [x] 页面提供概览、当前实例请求、慢请求明细及阶段详情；过滤模型/账号/API Token/结果/时间，按慢请求排序。
- [x] 复用 primitives 和 SVG，不引入依赖；null 与真实 0、无进展与思考、未知结果与失败有明确显示区别。
- [x] active 5 秒、overview 60 秒轮询；history 手动/筛选刷新，隐藏/卸载暂停，取消旧查询，失败保留旧值及更新时间。
- [x] 本地浏览器验证 30 分钟心跳场景可用短时 fixture 展示等价状态，记录实际操作，不伪装生产根因已定位。

## 5. 验证与交付

实现后按仓库要求依次运行，文档规划阶段不运行这些命令：

```bash
cd web
npm run build
cd ..
cargo fmt --check
cargo test --lib
cargo check
```

若 web 依赖尚不存在，先按 lockfile 执行 `npm ci`。只在本次明确改动、失败或仍有风险时扩展/重复检查，不调用真实上游性能脚本。

- [x] 质量门覆盖 AC1–AC8；检查新增监控操作不在 poll/drop 中 await I/O。
- [x] 阅读最终 diff，检查 429/403/5xx、OAuth fallback、quota、response headers/body、日志脱敏和现有计费相关回归。
- [x] 更新 README 环境开关、口径、30 日保留与当前实例限制；更新相关 gateway/frontend spec。不要改写无关 Trellis runtime。
- [x] 记录新增 schema 的前向兼容和关闭采集/应用回滚办法。
- [x] 报告已通过的命令、未执行的 PG/浏览器项及实际原因；不自动部署或提交无关未跟踪文件。

## 主要风险与回退点

| 风险 | 验证/回退 |
| --- | --- |
| guard 提前结束、重复终态或 slot 泄漏 | 请求取消/EOF/drop、429 后台 drain、usage early-finalize 回归；关闭采集开关 |
| 心跳或元信息误算首字 | 特定 SSE fixture 验证首包、首内容、首正文分别更新 |
| 长请求尚未完成时看不到 | active registry 与历史分离；本地虚拟时间/慢流 fixture |
| SQLite 查询/写入导致额外延迟 | 有界批写、分页/范围限制、低频聚合，检查查询计划；关闭采集保留已写历史 |
| 数据丢失或未知被显示成成功/零 | health counters、unknown/aborted、sample_count/null 契约测试 |
| 多实例误报全站活跃数 | 明确 scope=instance/instance_id，UI 同步标识 |

## 验收结果（2026-09-24）

- `npm --prefix web run build`：通过（vue-tsc + Vite）。
- `cargo test --lib --quiet`：230 passed，0 failed，1 ignored（隔离 PG 项，已另行执行）。
- `PERFORMANCE_TEST_POSTGRES_DSN=<temporary instance> cargo test --lib postgres_performance_queries_match_sqlite -- --ignored --nocapture`：1 passed；临时 PostgreSQL 15、专用 schema，覆盖同一批 SQLite fixture 的迁移、nullable 参数、统计、分页、维度及清理。
- `cargo check --locked`、`cargo fmt --check`、`git diff --check`：通过。
- 本机临时 SQLite 服务：性能所有查询接口无认证均 401；非法时间/分页/排序 400；未认证推理失败仍写入性能记录；32 分钟样本进入 >=30min 档并排在慢请求首位，详情可查。
- Chrome/agent-browser：登录、导航、直接刷新 /performance、模型筛选、当前进行中请求、慢请求与详情、故障保留旧值均通过；手机 390px 页面实际 scrollWidth=390，修复表格溢出；控制台无页面错误。
- 30 分钟心跳、thinking/text/tool、压缩原字节、SSE error+HTTP200、EOF/drop/abort、writer 失败和 headers/trailers/size_hint 通过确定性本地测试；浏览器活跃列表通过尚未完成的本地请求体上传验证，未调用真实模型。
- 无外部线上部署；临时浏览器、网关与 PostgreSQL 已停止。尚未判断 2026-09-24 线上慢响应具体根因。
- SQLx Any 0.7 的 NULL 编码/占位符行为已写入 performance-monitoring spec；src/store/token_store.rs 与 src/tlsfp/tlsfp.rs 仅为既有 rustfmt 差异调整。
- 完整检查与记录已完成。用户已批准提交并推送发布；完成任务归档，本地追加 journal（既有日志未跟踪，不自动纳入发布）。
