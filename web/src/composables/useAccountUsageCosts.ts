import { onBeforeUnmount, shallowRef } from 'vue'
import { api, type UsageReport } from '@/api'
import { usageDateRange } from '@/lib/usage'

export type AccountCostPeriod = 'today' | 'last_30_days'

export interface AccountUsageCost {
  knownCostNanoUsd: string
  costComplete: boolean
}

const ZERO_COST: AccountUsageCost = {
  knownCostNanoUsd: '0',
  costComplete: true,
}

function costsByAccount(report: UsageReport): Map<number, AccountUsageCost> {
  return new Map(
    report.breakdown.map((row) => [
      Number(row.key),
      {
        knownCostNanoUsd: row.metrics.known_cost_nano_usd,
        costComplete: row.metrics.cost_complete,
      },
    ]),
  )
}

function usageQuery(period: AccountCostPeriod) {
  const range = usageDateRange(period)
  return {
    granularity: 'day' as const,
    start_date: range.startDate,
    end_date: range.endDate,
    group_by: 'account' as const,
  }
}

export function useAccountUsageCosts() {
  const todayCosts = shallowRef(new Map<number, AccountUsageCost>())
  const last30DayCosts = shallowRef(new Map<number, AccountUsageCost>())
  const loaded = shallowRef<Record<AccountCostPeriod, boolean>>({
    today: false,
    last_30_days: false,
  })
  const loading = shallowRef(false)
  let controller: AbortController | null = null

  async function loadAccountUsageCosts() {
    controller?.abort()
    const currentController = new AbortController()
    controller = currentController
    loading.value = true

    const results = await Promise.allSettled([
      api.getUsage(usageQuery('today'), currentController.signal),
      api.getUsage(usageQuery('last_30_days'), currentController.signal),
    ])

    if (controller !== currentController) return
    const nextLoaded = { ...loaded.value }
    const todayResult = results[0]
    if (todayResult.status === 'fulfilled') {
      todayCosts.value = costsByAccount(todayResult.value)
      nextLoaded.today = true
    }
    const last30DayResult = results[1]
    if (last30DayResult.status === 'fulfilled') {
      last30DayCosts.value = costsByAccount(last30DayResult.value)
      nextLoaded.last_30_days = true
    }
    loaded.value = nextLoaded
    loading.value = false
  }

  function accountCost(accountId: number, period: AccountCostPeriod): AccountUsageCost | null {
    if (!loaded.value[period]) return null
    const costs = period === 'today' ? todayCosts.value : last30DayCosts.value
    return costs.get(accountId) ?? ZERO_COST
  }

  onBeforeUnmount(() => {
    controller?.abort()
    controller = null
  })

  return {
    loading,
    accountCost,
    loadAccountUsageCosts,
  }
}
