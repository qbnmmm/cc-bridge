export type UsageDatePreset = 'today' | 'this_week' | 'this_month' | 'last_30_days' | 'custom'
export type UsageDateShortcut = Exclude<UsageDatePreset, 'custom'>

export interface UsageDateRange {
  startDate: string
  endDate: string
}

const MINIMUM_DAILY_TREND_POINTS = 7

const singaporeDateFormatter = new Intl.DateTimeFormat('en-CA', {
  timeZone: 'Asia/Singapore',
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
})

export function singaporeDate(offsetDays = 0): string {
  return singaporeDateFormatter.format(new Date(Date.now() + offsetDays * 86_400_000))
}

function shiftDate(dateString: string, offsetDays: number): string {
  const date = new Date(`${dateString}T00:00:00Z`)
  date.setUTCDate(date.getUTCDate() + offsetDays)
  return date.toISOString().slice(0, 10)
}

function startOfSingaporeWeek(today: string): string {
  const date = new Date(`${today}T00:00:00Z`)
  const daysSinceMonday = (date.getUTCDay() + 6) % 7
  date.setUTCDate(date.getUTCDate() - daysSinceMonday)
  return date.toISOString().slice(0, 10)
}

export function usageDateRange(preset: UsageDateShortcut): UsageDateRange {
  const today = singaporeDate()
  switch (preset) {
    case 'today':
      return { startDate: today, endDate: today }
    case 'this_week':
      return { startDate: startOfSingaporeWeek(today), endDate: today }
    case 'this_month':
      return { startDate: `${today.slice(0, 7)}-01`, endDate: today }
    case 'last_30_days':
      return { startDate: shiftDate(today, -29), endDate: today }
  }
}

export function dailyTrendDateRange(range: UsageDateRange): UsageDateRange | null {
  const start = new Date(`${range.startDate}T00:00:00Z`)
  const end = new Date(`${range.endDate}T00:00:00Z`)
  const pointCount = Math.floor((end.getTime() - start.getTime()) / 86_400_000) + 1
  if (pointCount <= 0 || pointCount >= MINIMUM_DAILY_TREND_POINTS) return null

  const retentionStart = shiftDate(singaporeDate(), -364)
  const desiredStart = shiftDate(range.startDate, pointCount - MINIMUM_DAILY_TREND_POINTS)
  const startDate = desiredStart < retentionStart ? retentionStart : desiredStart
  return startDate === range.startDate ? null : { startDate, endDate: range.endDate }
}

export function formatNanoUsd(nanoUsd: string, maxFractionDigits = 6): string {
  const value = BigInt(nanoUsd)
  const sign = value < 0n ? '-' : ''
  const absolute = value < 0n ? -value : value
  const whole = absolute / 1_000_000_000n
  const fractionNano = absolute % 1_000_000_000n
  const threshold = 10n ** BigInt(9 - maxFractionDigits)

  if (whole === 0n && fractionNano > 0n && fractionNano < threshold) {
    return `${sign}<$${`0.${'0'.repeat(maxFractionDigits - 1)}1`}`
  }

  const fraction = fractionNano
    .toString()
    .padStart(9, '0')
    .slice(0, maxFractionDigits)
    .replace(/0+$/, '')
  return `${sign}$${whole.toLocaleString('en-US')}${fraction ? `.${fraction}` : ''}`
}

export function nanoUsdToUsd(nanoUsd: string): number {
  return Number(BigInt(nanoUsd)) / 1_000_000_000
}
