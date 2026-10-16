import type { Metric, TrayEntry } from '@/lib/types'

import { describe, expect, it } from 'vitest'

import { defaultSettings } from '@/lib/settings'

import { cardLayout } from './layout'

function meter(id: string, label = id): Metric {
  return {
    id,
    label,
    kind: 'meter',
    used: 10,
    limit: 100,
    unit: { type: 'percent' },
    resetsAt: null,
    periodSecs: null,
    notStarted: false,
  }
}

function entry(app: TrayEntry['app'], metrics: Array<Metric>): TrayEntry {
  return { id: 'p1', app, name: 'Work', command: 'claude-work', plan: null, usage: null, metrics, error: null }
}

function ids(placed: Array<{ id: string }>) {
  return placed.map((slot) => slot.id)
}

describe('cardLayout', () => {
  it('lays a Claude card out in its default order', () => {
    const layout = cardLayout(
      entry('claude', [
        meter('extra'),
        meter('weekly.sonnet'),
        meter('weekly'),
        meter('resets'),
        meter('weekly.fable'),
        meter('session'),
      ]),
      defaultSettings.layout,
    )
    expect(ids(layout.alwaysVisible)).toEqual(['session', 'weekly', 'weekly.fable', 'extra'])
    expect(ids(layout.onDemand)).toEqual(['weekly.sonnet', 'resets'])
  })

  it('keeps Spark on demand for Codex', () => {
    const layout = cardLayout(
      entry('codex', [meter('weekly'), meter('spark'), meter('credits')]),
      defaultSettings.layout,
    )
    expect(ids(layout.alwaysVisible)).toEqual(['weekly', 'credits'])
    expect(ids(layout.onDemand)).toEqual(['spark'])
  })

  it('follows the layout the user chose, placed metrics without data included', () => {
    const layout = cardLayout(entry('claude', [meter('session', 'Session'), meter('weekly', 'Weekly')]), {
      ...defaultSettings.layout,
      profiles: { p1: { alwaysVisible: ['weekly', 'today'], onDemand: ['weekly.fable'] } },
    })
    expect(ids(layout.alwaysVisible)).toEqual(['weekly', 'today'])
    expect(layout.alwaysVisible[1]).toEqual({ id: 'today', label: 'Today', metric: null })
    expect(layout.onDemand[0]?.label).toBe('Weekly · Fable')
  })
})
