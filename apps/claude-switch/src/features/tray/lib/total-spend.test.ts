import type { Metric, TrayEntry } from '@/lib/types'

import { describe, expect, it } from 'vitest'

import { costPerMillion, hasSpend, shareOf, totalSpend } from './total-spend'

function spend(id: string, cost: number, tokens: number): Metric {
  return {
    id,
    label: id,
    kind: 'values',
    values: [
      { amount: cost, unit: { type: 'money', currency: 'USD' } },
      { amount: tokens, unit: { type: 'tokens' } },
    ],
  }
}

function entry(id: string, metrics: Array<Metric>): TrayEntry {
  return { id, app: 'claude', name: id, command: id, plan: null, usage: null, metrics, error: null }
}

describe('totalSpend', () => {
  it('adds up the period, a slice per profile that spent, coloured by its place in the list', () => {
    const entries = [
      entry('a', [spend('today', 6, 2_000_000)]),
      entry('b', [spend('yesterday', 1, 10)]),
      entry('c', [spend('today', 2, 6_000_000)]),
    ]
    const today = totalSpend(entries, 'today')
    expect(today.slices.map((slice) => [slice.id, slice.slot])).toEqual([
      ['a', 1],
      ['c', 3],
    ])
    expect(today.cost).toBe(8)
    expect(today.tokens).toBe(8_000_000)
    expect(shareOf(today.slices[0]!, today, 'cost')).toBe(0.75)
    expect(shareOf(today.slices[0]!, today, 'tokens')).toBe(0.25)
    expect(costPerMillion(today.cost, today.tokens)).toBe(1)
  })

  it('folds profiles past the sixth into Other', () => {
    const entries = Array.from({ length: 8 }, (_, index) => entry(`p${index}`, [spend('last30', 1, 1)]))
    const slices = totalSpend(entries, 'last30Days').slices
    expect(slices).toHaveLength(7)
    expect(slices[slices.length - 1]).toMatchObject({ id: 'other', cost: 2, slot: null })
  })

  it('shows only when some profile has spend, and divides by nothing as nothing', () => {
    expect(hasSpend([entry('a', [])])).toBe(false)
    expect(hasSpend([entry('a', [spend('yesterday', 1, 1)])])).toBe(true)
    expect(costPerMillion(1, 0)).toBeNull()
  })
})
