import type { AppId } from '@/lib/app-registry'
import type { Metric, Settings, TrayEntry } from '@/lib/types'

/**
 * A metric the card should show: its data, or only its id and a label when
 * the user placed it but the profile reports nothing for it.
 */
export type Placed = { id: string; label: string; metric: Metric | null }

export type CardLayout = {
  alwaysVisible: Array<Placed>
  onDemand: Array<Placed>
}

/**
 * Each app's metrics in the order its card shows them by default, with where
 * each sits. `weekly.*` stands for the per-model weekly windows.
 */
const defaults: Record<AppId, Array<[string, 'alwaysVisible' | 'onDemand']>> = {
  claude: [
    ['session', 'alwaysVisible'],
    ['weekly', 'alwaysVisible'],
    ['weekly.*', 'alwaysVisible'],
    ['trend', 'alwaysVisible'],
    ['extra', 'alwaysVisible'],
    ['today', 'onDemand'],
    ['yesterday', 'onDemand'],
    ['last30', 'onDemand'],
    ['resets', 'onDemand'],
  ],
  codex: [
    ['session', 'alwaysVisible'],
    ['weekly', 'alwaysVisible'],
    ['trend', 'alwaysVisible'],
    ['credits', 'alwaysVisible'],
    ['spark', 'onDemand'],
    ['spark.weekly', 'onDemand'],
    ['today', 'onDemand'],
    ['yesterday', 'onDemand'],
    ['last30', 'onDemand'],
    ['resets', 'onDemand'],
  ],
}

/**
 * Per-model weekly windows that start on demand rather than always visible:
 * the older models a plan still reports.
 */
const quietScopedWeekly = new Set(['weekly.sonnet', 'weekly.opus'])

function defaultSlot(app: AppId, id: string): { order: number; place: 'alwaysVisible' | 'onDemand' } {
  const order = defaults[app].findIndex(([pattern]) =>
    pattern.endsWith('.*') ? id.startsWith(pattern.slice(0, -1)) && id !== 'weekly' : pattern === id,
  )
  if (order === -1) {
    return { order: defaults[app].length, place: 'onDemand' }
  }
  const place = quietScopedWeekly.has(id) ? 'onDemand' : (defaults[app][order]?.[1] ?? 'onDemand')
  return { order, place }
}

/**
 * What the card for `entry` shows, and where: the user's layout for it if
 * they made one (a placed metric with no data shows as such), else its app's
 * default for the metrics it reports.
 */
export function cardLayout(entry: TrayEntry, layout: Settings['layout']): CardLayout {
  const chosen = layout.profiles[entry.id]
  const find = (id: string): Placed => {
    const metric = entry.metrics.find((candidate) => candidate.id === id) ?? null
    return { id, label: metric?.label ?? labelFor(id), metric }
  }
  if (chosen) {
    return { alwaysVisible: chosen.alwaysVisible.map(find), onDemand: chosen.onDemand.map(find) }
  }
  const slotted = entry.metrics
    .map((metric, index) => ({ metric, index, ...defaultSlot(entry.app, metric.id) }))
    .sort((a, b) => a.order - b.order || a.index - b.index)
  const placed = (place: 'alwaysVisible' | 'onDemand') =>
    slotted.filter((slot) => slot.place === place).map(({ metric }) => ({ id: metric.id, label: metric.label, metric }))
  return { alwaysVisible: placed('alwaysVisible'), onDemand: placed('onDemand') }
}

const labels: Record<string, string> = {
  session: 'Session',
  weekly: 'Weekly',
  extra: 'Extra usage',
  credits: 'Credits',
  resets: 'Rate limit resets',
  spark: 'Spark',
  'spark.weekly': 'Spark weekly',
  today: 'Today',
  yesterday: 'Yesterday',
  last30: 'Last 30 days',
  trend: 'Usage trend',
}

/**
 * A label for a metric the profile doesn't report, from its id.
 */
function labelFor(id: string): string {
  if (labels[id]) {
    return labels[id]
  }
  if (id.startsWith('weekly.')) {
    const model = id.slice('weekly.'.length)
    return `Weekly · ${model.charAt(0).toUpperCase()}${model.slice(1)}`
  }
  return id
}
