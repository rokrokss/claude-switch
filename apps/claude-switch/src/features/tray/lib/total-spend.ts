import type { Settings, TrayEntry } from '@/lib/types'

type TotalSpendSettings = Settings['display']['totalSpend']
type Period = TotalSpendSettings['period']

/**
 * Profiles past this many share one `Other` slice: the series palette has
 * six colours, and more slices than that can't be told apart.
 */
const maxSlices = 6

const metricOfPeriod: Record<Period, string> = {
  today: 'today',
  yesterday: 'yesterday',
  last30Days: 'last30',
}

export type SpendSlice = {
  /**
   * The profile's id, or `other` for the profiles folded together.
   */
  id: string
  name: string
  cost: number
  tokens: number
  /**
   * The series colour, 1 to 6, kept by the profile's place in the list so
   * it doesn't change with the period; `null` for Other.
   */
  slot: number | null
}

export type TotalSpend = {
  slices: Array<SpendSlice>
  cost: number
  tokens: number
}

/**
 * Whether any profile has spend read from its logs, in any period: the card
 * shows only then.
 */
export function hasSpend(entries: Array<TrayEntry>): boolean {
  const ids = Object.values(metricOfPeriod)
  return entries.some((entry) => entry.metrics.some((metric) => ids.includes(metric.id)))
}

/**
 * Every profile's spend in `period`, a slice each in list order.
 */
export function totalSpend(entries: Array<TrayEntry>, period: Period): TotalSpend {
  const slices: Array<SpendSlice> = []
  const other: SpendSlice = { id: 'other', name: 'Other', cost: 0, tokens: 0, slot: null }
  entries.forEach((entry, index) => {
    const spend = spendOf(entry, metricOfPeriod[period])
    if (!spend) {
      return
    }
    if (index < maxSlices) {
      slices.push({ id: entry.id, name: entry.name, ...spend, slot: index + 1 })
    } else {
      other.cost += spend.cost
      other.tokens += spend.tokens
    }
  })
  if (other.cost > 0 || other.tokens > 0) {
    slices.push(other)
  }
  return {
    slices,
    cost: slices.reduce((sum, slice) => sum + slice.cost, 0),
    tokens: slices.reduce((sum, slice) => sum + slice.tokens, 0),
  }
}

function spendOf(entry: TrayEntry, metricId: string): { cost: number; tokens: number } | null {
  const metric = entry.metrics.find((candidate) => candidate.id === metricId)
  if (metric?.kind !== 'values') {
    return null
  }
  const amountIn = (type: 'money' | 'tokens') =>
    metric.values.find((reading) => reading.unit.type === type)?.amount ?? 0
  return { cost: amountIn('money'), tokens: amountIn('tokens') }
}

/**
 * Dollars per million tokens, or `null` without tokens to divide by.
 */
export function costPerMillion(cost: number, tokens: number): number | null {
  return tokens > 0 ? cost / (tokens / 1e6) : null
}

/**
 * A slice's share of the whole, 0 to 1: of tokens when they are what's
 * shown, otherwise of cost.
 */
export function shareOf(slice: SpendSlice, total: TotalSpend, metric: TotalSpendSettings['metric']): number {
  const [part, whole] = metric === 'tokens' ? [slice.tokens, total.tokens] : [slice.cost, total.cost]
  return whole > 0 ? part / whole : 0
}
