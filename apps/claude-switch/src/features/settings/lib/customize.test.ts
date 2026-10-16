import type { Metric, TrayEntry } from '@/lib/types'

import { describe, expect, it } from 'vitest'

import { defaultSettings } from '@/lib/settings'

import { customizeRows, layoutOf, moveTo, reorder, withoutLayout } from './customize'

function meter(id: string): Metric {
  return {
    id,
    label: id,
    kind: 'meter',
    used: 1,
    limit: 100,
    unit: { type: 'percent' },
    resetsAt: null,
    periodSecs: null,
    notStarted: false,
  }
}

const entry: TrayEntry = {
  id: 'p1',
  app: 'claude',
  name: 'Work',
  command: 'claude-work',
  plan: null,
  usage: null,
  metrics: [meter('session'), meter('weekly'), meter('resets'), meter('extra')],
  error: null,
}

const places = (rows: ReturnType<typeof customizeRows>) => rows.map((row) => `${row.id}:${row.place}`)

describe('customize', () => {
  it('lists the default layout, then what is off', () => {
    expect(places(customizeRows(entry, defaultSettings.layout))).toEqual([
      'session:alwaysVisible',
      'weekly:alwaysVisible',
      'extra:alwaysVisible',
      'resets:onDemand',
    ])
    const chosen = {
      ...defaultSettings.layout,
      profiles: { p1: { alwaysVisible: ['weekly', 'today'], onDemand: [] } },
    }
    const rows = customizeRows(entry, chosen)
    expect(places(rows)).toEqual([
      'weekly:alwaysVisible',
      'today:alwaysVisible',
      'session:off',
      'resets:off',
      'extra:off',
    ])
    expect(rows.find((row) => row.id === 'today')?.reported).toBe(false)
  })

  it('moves a metric between groups and reorders within one', () => {
    const rows = customizeRows(entry, defaultSettings.layout)
    const moved = moveTo(rows, 'session', 'onDemand')
    expect(layoutOf(moved)).toEqual({ alwaysVisible: ['weekly', 'extra'], onDemand: ['resets', 'session'] })
    const reordered = reorder(moved, 'extra', 'weekly')
    expect(layoutOf(reordered).alwaysVisible).toEqual(['extra', 'weekly'])
    // Across groups, a drag changes nothing.
    expect(reorder(moved, 'extra', 'resets')).toBe(moved)
  })

  it('drops an entry layout to go back to the default', () => {
    const layout = { ...defaultSettings.layout, profiles: { p1: { alwaysVisible: [], onDemand: [] } } }
    expect(withoutLayout(layout, 'p1').profiles).toEqual({})
  })
})
