import type { PerformanceOutcome } from '../api'

export const outcomeLabels: Record<PerformanceOutcome, string> = {
  success: '完成', local_error: '本地拒绝', http_error: '上游 HTTP 错误', send_error: '发送失败',
  send_timeout: '发送超时', read_timeout: '读取超时', stream_error: '流错误', incomplete: '流未完整结束',
  aborted: '取消 / 响应释放', unknown: '结果未知',
}
export const phaseLabels: Record<string, string> = {
  preparing: '请求准备', waiting_upstream: '等待上游响应', waiting_content: '等待首内容',
  thinking: '思考中', text: '正文输出', tool: '工具参数输出', finishing: '等待流结束',
}
export function formatMs(ms: number | null | undefined): string {
  if (ms == null) return '—'
  if (ms < 1000) return `${Math.round(ms)} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} 秒`
  return `${(ms / 60_000).toFixed(1)} 分钟`
}
export function formatSpeed(rate: number | null | undefined): string {
  return rate == null ? '—' : `${rate.toFixed(1)} tok/s`
}
export function formatTime(time: string | number): string {
  return new Intl.DateTimeFormat('zh-CN', { timeZone: 'Asia/Singapore', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false }).format(new Date(time))
}
