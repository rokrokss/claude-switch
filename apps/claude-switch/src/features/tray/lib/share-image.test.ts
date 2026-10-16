import type { Metric, TrayEntry } from '@/lib/types'

import { describe, expect, it } from 'vitest'

import { defaultSettings } from '@/lib/settings'

import { profileShareCard, totalSpendShareCard } from './share-image'
import { totalSpend } from './total-spend'

const now = Date.parse('2026-10-17T12:00:00Z')

const session: Metric = {
  id: 'session',
  label: 'Session',
  kind: 'meter',
  used: 30,
  limit: 100,
  unit: { type: 'percent' },
  resetsAt: new Date(now + 2 * 3_600_000 + 60_000).toISOString(),
  periodSecs: 5 * 3600,
  notStarted: false,
  state: { kind: 'healthy', projectedFraction: 0.5 },
}

const today: Metric = {
  id: 'today',
  label: 'Today',
  kind: 'values',
  values: [
    { amount: 3, unit: { type: 'money', currency: 'USD' } },
    { amount: 1_000_000, unit: { type: 'tokens' } },
  ],
}

const trend: Metric = {
  id: 'trend',
  label: 'Usage trend',
  kind: 'chart',
  unit: { type: 'tokens' },
  points: [
    { date: '2026-10-16', value: 4 },
    { date: '2026-10-17', value: 2 },
  ],
}

const entry: TrayEntry = {
  id: 'work',
  app: 'claude',
  name: 'Work',
  command: 'claude-work',
  plan: 'Max 20x',
  usage: null,
  metrics: [session, today, trend],
  error: null,
}

describe('profileShareCard', () => {
  it('draws what the card always shows, as the panel reads it', () => {
    const card = profileShareCard(entry, defaultSettings, now)
    expect(card).toMatchObject({ title: 'Work', badge: 'Max 20x', aside: 'Claude' })
    // Today is on demand by default, so it isn't shared.
    expect(card.lines).toEqual([
      {
        kind: 'meter',
        label: 'Session',
        headline: '30% used',
        fraction: 0.3,
        tone: 'normal',
        note: 'Resets in 2h 1m',
      },
      { kind: 'bars', label: 'Usage trend', values: [4, 2] },
    ])
  })
})

describe('totalSpendShareCard', () => {
  it('names the period, then the total, the bar and each profile', () => {
    const total = totalSpend([entry, { ...entry, id: 'home', name: 'Home' }], 'today')
    const card = totalSpendShareCard(total, defaultSettings.display.totalSpend)
    expect(card.badge).toBe('Today')
    expect(card.lines[0]).toEqual({ kind: 'hero', value: expect.stringMatching(/6\.00$/) })
    expect(card.lines[1]).toEqual({
      kind: 'stack',
      segments: [
        { share: 0.5, slot: 1 },
        { share: 0.5, slot: 2 },
      ],
    })
    expect(card.lines[3]).toMatchObject({ kind: 'legend', name: 'Home', share: '50%', slot: 2 })
  })
})
