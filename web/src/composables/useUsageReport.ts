import { computed, onBeforeUnmount, onMounted, reactive, shallowRef } from 'vue'
import {
  api,
  type UsageDimensions,
  type UsageGranularity,
  type UsageGroupBy,
  type UsageQueryParams,
  type UsageReport,
} from '@/api'
import {
  usageDateRange,
  type UsageDatePreset,
  type UsageDateShortcut,
} from '@/lib/usage'

export function useUsageReport() {
  const initialDateRange = usageDateRange('today')
  const filters = reactive<UsageQueryParams>({
    granularity: 'day',
    start_date: initialDateRange.startDate,
    end_date: initialDateRange.endDate,
    group_by: 'model',
  })
  const report = shallowRef<UsageReport | null>(null)
  const dimensions = shallowRef<UsageDimensions>({ accounts: [], api_tokens: [], models: [] })
  const loading = shallowRef(false)
  const error = shallowRef('')
  const datePreset = shallowRef<UsageDatePreset>('today')
  let controller: AbortController | null = null

  const hasIngestionGap = computed(() => {
    const health = report.value?.ingestion
    return Boolean(
      health &&
        (health.queue_dropped_total > 0 ||
          health.write_failed_total > 0 ||
          health.parse_failed_total > 0 ||
          health.parse_oversize_total > 0),
    )
  })
  async function loadDimensions() {
    try {
      dimensions.value = await api.getUsageDimensions()
    } catch {
      // Report remains usable without dimension labels.
    }
  }

  async function fetchReport(clearCurrent: boolean) {
    controller?.abort()
    const currentController = new AbortController()
    controller = currentController
    if (clearCurrent) report.value = null
    loading.value = true
    error.value = ''
    try {
      report.value = await api.getUsage({ ...filters }, currentController.signal)
    } catch (cause) {
      if ((cause as Error).name !== 'AbortError') {
        error.value = (cause as Error).message || '加载用量失败'
      }
    } finally {
      if (controller === currentController) loading.value = false
    }
  }

  function loadReport() {
    return fetchReport(false)
  }

  function applyFilters(value: Partial<UsageQueryParams>) {
    if ('start_date' in value || 'end_date' in value) datePreset.value = 'custom'
    Object.assign(filters, value)
    void fetchReport(true)
  }

  function setGranularity(value: UsageGranularity) {
    applyFilters({ granularity: value })
  }

  function setGroupBy(value: UsageGroupBy) {
    applyFilters({ group_by: value })
  }

  function setDatePreset(value: UsageDateShortcut) {
    const range = usageDateRange(value)
    datePreset.value = value
    Object.assign(filters, { start_date: range.startDate, end_date: range.endDate })
    void fetchReport(true)
  }

  onMounted(() => {
    void Promise.all([loadDimensions(), loadReport()])
  })
  onBeforeUnmount(() => controller?.abort())

  return {
    filters,
    report,
    dimensions,
    loading,
    error,
    hasIngestionGap,
    datePreset,
    loadReport,
    applyFilters,
    setGranularity,
    setGroupBy,
    setDatePreset,
  }
}
