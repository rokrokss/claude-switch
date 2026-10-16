import type { Settings, TrayEntry } from '@/lib/types'

import { cardLayout } from '@/features/tray/lib/layout'

export type Place = 'alwaysVisible' | 'onDemand' | 'off'

export type CustomizeRow = {
  id: string
  label: string
  place: Place
  /**
   * Whether the profile reports it now; a metric placed earlier may not.
   */
  reported: boolean
}

/**
 * Every metric the profile's card can show, where it sits now: placed ones
 * in their order, then the ones it reports that are off.
 */
export function customizeRows(entry: TrayEntry, layout: Settings['layout']): Array<CustomizeRow> {
  const card = cardLayout(entry, layout)
  const placed = [
    ...card.alwaysVisible.map((slot) => ({ ...slot, place: 'alwaysVisible' as const })),
    ...card.onDemand.map((slot) => ({ ...slot, place: 'onDemand' as const })),
  ]
  const off = entry.metrics
    .filter((metric) => !placed.some((slot) => slot.id === metric.id))
    .map((metric) => ({ id: metric.id, label: metric.label, metric, place: 'off' as const }))
  return [...placed, ...off].map((slot) => ({
    id: slot.id,
    label: slot.label,
    place: slot.place,
    reported: slot.metric !== null,
  }))
}

/**
 * The layout the rows describe: each placement's ids, in order.
 */
export function layoutOf(rows: Array<CustomizeRow>): { alwaysVisible: Array<string>; onDemand: Array<string> } {
  const ids = (place: Place) => rows.filter((row) => row.place === place).map((row) => row.id)
  return { alwaysVisible: ids('alwaysVisible'), onDemand: ids('onDemand') }
}

/**
 * The rows with one moved to `place`, at the end of its new group.
 */
export function moveTo(rows: Array<CustomizeRow>, id: string, place: Place): Array<CustomizeRow> {
  const row = rows.find((candidate) => candidate.id === id)
  if (!row || row.place === place) {
    return rows
  }
  return [...rows.filter((candidate) => candidate.id !== id), { ...row, place }]
}

/**
 * The rows with `activeId` moved to where `overId` was, within one group.
 */
export function reorder(rows: Array<CustomizeRow>, activeId: string, overId: string): Array<CustomizeRow> {
  const from = rows.findIndex((row) => row.id === activeId)
  const to = rows.findIndex((row) => row.id === overId)
  if (from === -1 || to === -1 || rows[from]?.place !== rows[to]?.place) {
    return rows
  }
  const next = [...rows]
  const [moved] = next.splice(from, 1)
  if (moved) {
    next.splice(to, 0, moved)
  }
  return next
}

/**
 * The layout settings with the entry's own layout dropped: its card goes
 * back to its app's default.
 */
export function withoutLayout(layout: Settings['layout'], entryId: string): Settings['layout'] {
  const { [entryId]: _dropped, ...profiles } = layout.profiles
  return { ...layout, profiles }
}
