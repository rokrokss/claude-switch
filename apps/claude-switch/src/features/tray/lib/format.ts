import type { Metric, MetricUnit, Settings } from '@/lib/types'

type Meter = Extract<Metric, { kind: 'meter' }>
type TimeFormat = Settings['display']['timeFormat']

const minuteMs = 60_000
const hourMs = 60 * minuteMs
const dayMs = 24 * hourMs
/**
 * A deadline this close, or already past, reads as "soon".
 */
const soonMs = 5 * minuteMs

/**
 * A time left, rounded up to the minute: `2d 4h`, `3h 25m`, `12m`, or `soon`
 * within `soon` (five minutes unless given).
 */
export function formatDuration(ms: number, soon = soonMs): string {
  if (ms <= soon) {
    return 'soon'
  }
  const minutes = Math.ceil(ms / minuteMs)
  const days = Math.floor(minutes / (24 * 60))
  const hours = Math.floor((minutes % (24 * 60)) / 60)
  const rest = minutes % 60
  if (days > 0) {
    return `${days}d ${hours}h`
  }
  if (hours > 0) {
    return `${hours}h ${rest}m`
  }
  return `${rest}m`
}

/**
 * A moment as a day and a time: `today at 18:38`, `tomorrow at 9:05 AM`,
 * `Feb 15 at 18:38`.
 */
export function formatMoment(at: Date, timeFormat: TimeFormat, now: Date): string {
  const time = new Intl.DateTimeFormat(undefined, {
    hour: 'numeric',
    minute: '2-digit',
    ...(timeFormat === 'auto' ? {} : { hour12: timeFormat === '12h' }),
  }).format(at)
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
  const dayOffset = Math.floor((at.getTime() - startOfToday) / dayMs)
  if (dayOffset === 0) {
    return `today at ${time}`
  }
  if (dayOffset === 1) {
    return `tomorrow at ${time}`
  }
  const day = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' }).format(at)
  return `${day} at ${time}`
}

/**
 * When something happens, as a countdown (`Resets in 3h 25m`) or as the
 * moment itself (`Resets today at 18:38`).
 */
export function formatDeadline(
  verb: string,
  at: Date,
  mode: Settings['display']['resetDisplay'],
  timeFormat: TimeFormat,
  now: Date,
): string {
  const ms = at.getTime() - now.getTime()
  if (mode === 'exact' && ms > soonMs) {
    return `${verb} ${formatMoment(at, timeFormat, now)}`
  }
  const duration = formatDuration(ms)
  return duration === 'soon' ? `${verb} soon` : `${verb} in ${duration}`
}

/**
 * Whole numbers below a thousand, then `12.9K`, `3.4M`, `1.2B`.
 */
export function formatCompact(amount: number): string {
  const abs = Math.abs(amount)
  if (abs >= 1e9) {
    return `${(amount / 1e9).toFixed(1)}B`
  }
  if (abs >= 1e6) {
    return `${(amount / 1e6).toFixed(1)}M`
  }
  if (abs >= 1e3) {
    return `${(amount / 1e3).toFixed(1)}K`
  }
  return `${Math.round(amount)}`
}

function formatMoney(amount: number, currency: string): string {
  try {
    return new Intl.NumberFormat(undefined, { style: 'currency', currency, currencyDisplay: 'narrowSymbol' }).format(
      amount,
    )
  } catch {
    // Intl throws on a currency code it doesn't know.
    return `${amount.toFixed(2)} ${currency}`
  }
}

/**
 * An amount in `unit`: `48%`, `$12.50`, `796`.
 */
function formatAmount(amount: number, unit: MetricUnit): string {
  if (unit.type === 'percent') {
    return `${Math.round(amount)}%`
  }
  if (unit.type === 'money') {
    return formatMoney(amount, unit.currency)
  }
  return formatCompact(amount)
}

/**
 * US dollars as spend reads them: `$12.34`.
 */
export function formatCost(amount: number): string {
  return formatMoney(amount, 'USD')
}

/**
 * A reading with what it counts, where that isn't plain: `1.2M tokens`.
 */
export function formatReading(amount: number, unit: MetricUnit): string {
  const text = formatAmount(amount, unit)
  return unit.type === 'tokens' ? `${text} tokens` : text
}

/**
 * A local day (`YYYY-MM-DD`) as `Oct 12`.
 */
export function formatDay(date: string): string {
  const [year = 0, month = 1, day = 1] = date.split('-').map(Number)
  return new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' }).format(new Date(year, month - 1, day))
}

/**
 * The share of a meter shown, used or left, 0 to 1.
 */
export function shownFraction(meter: Meter, meterStyle: Settings['display']['meterStyle']): number {
  if (meter.limit <= 0) {
    return 0
  }
  const used = Math.min(Math.max(meter.used / meter.limit, 0), 1)
  return meterStyle === 'left' ? 1 - used : used
}

/**
 * A meter's reading as the panel heads it: `52% left`, `48% used`,
 * `$12.50 left`.
 */
export function meterHeadline(meter: Meter, meterStyle: Settings['display']['meterStyle']): string {
  if (meter.unit.type === 'percent') {
    const percent = Math.round(shownFraction(meter, meterStyle) * 100)
    return `${percent}% ${meterStyle === 'left' ? 'left' : 'used'}`
  }
  const amount = meterStyle === 'left' ? Math.max(meter.limit - meter.used, 0) : meter.used
  return `${formatAmount(amount, meter.unit)} ${meterStyle === 'left' ? 'left' : 'used'}`
}

/**
 * A reading without a limit, as its row shows it: `$31.84 · 796 credits`,
 * `3 available`, `$4.08 · 1.2M tokens`.
 */
export function valuesText(metric: Extract<Metric, { kind: 'values' }>): string {
  if (metric.id === 'resets') {
    return `${formatCompact(metric.values[0]?.amount ?? 0)} available`
  }
  return metric.values
    .map((reading) => {
      const text = formatReading(reading.amount, reading.unit)
      return reading.unit.type === 'count' && metric.id === 'credits' ? `${text} credits` : text
    })
    .join(' · ')
}
