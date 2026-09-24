import { computed, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { api, type PerformanceActive, type PerformanceDimensions, type PerformanceEvent, type PerformanceOutcome, type PerformancePage, type PerformanceQuery, type PerformanceReport } from '../api'

export interface PerformanceFilters {
  range: 'hour' | 'today' | 'day' | 'week' | 'month'
  account: string
  token: string
  model: string
  instance: string
}
export function usePerformance() {
  const filters = ref<PerformanceFilters>({ range: 'hour', account: '', token: '', model: '', instance: '' })
  const historyFilters = ref<{ outcome: PerformanceOutcome | ''; stream: string; minSeconds: string; sort: 'duration_desc' | 'created_at_desc' }>({ outcome: '', stream: '', minSeconds: '', sort: 'duration_desc' })
  const page = ref(1)
  const activePage = ref(1)
  const overview = ref<PerformanceReport | null>(null)
  const active = ref<PerformanceActive | null>(null)
  const history = ref<PerformancePage | null>(null)
  const dimensions = ref<PerformanceDimensions>({ accounts: [], api_tokens: [], models: [], instances: [] })
  const selected = ref<PerformanceEvent | null>(null)
  const channels = ['overview', 'active', 'history', 'dimensions', 'detail'] as const
  type Channel = typeof channels[number]
  const errors = reactive<Record<Channel, string>>({ overview: '', active: '', history: '', dimensions: '', detail: '' })
  const loading = reactive<Record<Channel, boolean>>({ overview: false, active: false, history: false, dimensions: false, detail: false })
  const updated = reactive<Record<Channel, number>>({ overview: 0, active: 0, history: 0, dimensions: 0, detail: 0 })
  const controllers: Partial<Record<Channel, AbortController>> = {}
  let disposed = false
  let activeTimer: ReturnType<typeof setInterval> | undefined
  let overviewTimer: ReturnType<typeof setInterval> | undefined

  const common = computed<PerformanceQuery>(() => ({
    account_id: filters.value.account ? Number(filters.value.account) : undefined,
    api_token_id: filters.value.token ? Number(filters.value.token) : undefined,
    model: filters.value.model || undefined, instance_id: filters.value.instance || undefined,
  }))
  function historicalQuery(): PerformanceQuery {
    const now = Date.now()
    const sgMidnight = Math.floor((now + 28_800_000) / 86_400_000) * 86_400_000 - 28_800_000
    const starts = { hour: now - 3_600_000, today: sgMidnight, day: now - 86_400_000, week: now - 7 * 86_400_000, month: sgMidnight - 29 * 86_400_000 }
    return { ...common.value, start_at: new Date(starts[filters.value.range]).toISOString(), end_at: new Date(now).toISOString() }
  }
  async function load<T>(key: Channel, fetcher: (signal: AbortSignal) => Promise<T>, accept: (value: T) => void) {
    controllers[key]?.abort()
    const controller = new AbortController()
    controllers[key] = controller
    loading[key] = true
    try {
      const value = await fetcher(controller.signal)
      if (disposed || controller.signal.aborted) return
      accept(value); errors[key] = ''; updated[key] = Date.now()
    } catch (error) {
      if (!disposed && !controller.signal.aborted) errors[key] = error instanceof Error ? error.message : '请求失败'
    } finally {
      if (controllers[key] === controller) loading[key] = false
    }
  }
  function loadOverview() { return load('overview', signal => api.getPerformance(historicalQuery(), signal), value => { overview.value = value }) }
  function loadActive() { return load('active', signal => api.getPerformanceActive({ ...common.value, page: activePage.value }, signal), value => { active.value = value }) }
  function loadHistory() {
    const f = historyFilters.value
    return load('history', signal => api.getPerformanceRequests({ ...historicalQuery(), page: page.value,
      outcome: f.outcome || undefined, is_stream: f.stream === '' ? undefined : f.stream === 'true',
      min_duration_ms: f.minSeconds === '' ? undefined : Number(f.minSeconds) * 1000, sort: f.sort }, signal), value => { history.value = value })
  }
  function loadDimensions() { return load('dimensions', signal => api.getPerformanceDimensions(signal), value => { dimensions.value = value }) }
  function refresh() { void loadOverview(); void loadActive(); void loadHistory(); void loadDimensions() }
  function showDetail(event: PerformanceEvent) {
    controllers.detail?.abort()
    selected.value = event
    errors.detail = ''
    if (event.outcome) void load('detail', signal => api.getPerformanceDetail(event.request_id, signal), value => { selected.value = value })
  }
  function closeDetail() { controllers.detail?.abort(); selected.value = null; errors.detail = '' }
  watch(filters, () => { page.value = 1; activePage.value = 1; refresh() }, { deep: true })
  watch(historyFilters, () => { page.value = 1; void loadHistory() }, { deep: true })
  watch(page, () => { void loadHistory() })
  watch(activePage, () => { void loadActive() })
  function onVisibility() {
    if (document.hidden) { for (const controller of Object.values(controllers)) controller.abort() }
    else refresh()
  }
  onMounted(() => {
    refresh()
    activeTimer = setInterval(() => { if (!document.hidden && !loading.active) void loadActive() }, 5000)
    overviewTimer = setInterval(() => { if (!document.hidden && !loading.overview) void loadOverview() }, 60_000)
    document.addEventListener('visibilitychange', onVisibility)
  })
  onUnmounted(() => {
    disposed = true; clearInterval(activeTimer); clearInterval(overviewTimer)
    document.removeEventListener('visibilitychange', onVisibility)
    for (const controller of Object.values(controllers)) controller.abort()
  })
  return { filters, historyFilters, page, activePage, overview, active, history, dimensions, selected, errors, loading, updated, refresh, loadHistory, showDetail, closeDetail }
}
