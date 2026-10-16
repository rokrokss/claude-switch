import type { Metric } from '@/lib/types'

import { describe, expect, it } from 'vitest'

import {
  formatCompact,
  formatDeadline,
  formatDuration,
  formatMoment,
  meterHeadline,
  shownFraction,
  valuesText,
} from './format'

type Meter = Extract<Metric, { kind: 'meter' }>

const minute = 60_000
const hour = 60 * minute

function meter(overrides: Partial<Meter> = {}): Meter {
  return {
    id: 'session',
    label: 'Session',
    kind: 'meter',
    used: 48,
    limit: 100,
    unit: { type: 'percent' },
    resetsAt: null,
    periodSecs: null,
    notStarted: false,
    ...overrides,
  }
}

describe('formatDuration', () => {
  it('rounds up to the minute and drops the smaller unit past a day', () => {
    expect(formatDuration(3 * hour + 24 * minute + 1)).toBe('3h 25m')
    expect(formatDuration(12 * minute)).toBe('12m')
    expect(formatDuration(52 * hour)).toBe('2d 4h')
  })

  it('reads as soon within five minutes or once past', () => {
    expect(formatDuration(4 * minute)).toBe('soon')
    expect(formatDuration(-hour)).toBe('soon')
  })
})

describe('formatMoment and formatDeadline', () => {
  const now = new Date(2026, 9, 17, 12, 0)

  it('names today, tomorrow, or the date', () => {
    expect(formatMoment(new Date(2026, 9, 17, 18, 38), '24h', now)).toBe('today at 18:38')
    expect(formatMoment(new Date(2026, 9, 18, 9, 5), '24h', now)).toBe('tomorrow at 09:05')
    expect(formatMoment(new Date(2026, 9, 25, 18, 38), '24h', now)).toMatch(/at 18:38$/)
  })

  it('counts down or names the moment, as the setting says', () => {
    const at = new Date(now.getTime() + 3 * hour + 25 * minute)
    expect(formatDeadline('Resets', at, 'countdown', 'auto', now)).toBe('Resets in 3h 25m')
    expect(formatDeadline('Resets', at, 'exact', '24h', now)).toBe('Resets today at 15:25')
    expect(formatDeadline('Limit', new Date(now.getTime() + minute), 'exact', '24h', now)).toBe('Limit soon')
  })
})

describe('meters', () => {
  it('heads a meter by what is used or what is left', () => {
    expect(meterHeadline(meter(), 'used')).toBe('48% used')
    expect(meterHeadline(meter(), 'left')).toBe('52% left')
    expect(shownFraction(meter({ used: 130 }), 'used')).toBe(1)
  })

  it('heads a money meter in its currency', () => {
    const extra = meter({ id: 'extra', used: 12.5, limit: 50, unit: { type: 'money', currency: 'USD' } })
    expect(meterHeadline(extra, 'left')).toMatch(/37\.50 left$/)
  })
})

describe('valuesText', () => {
  it('reads credits as money and a count', () => {
    const credits: Extract<Metric, { kind: 'values' }> = {
      id: 'credits',
      label: 'Credits',
      kind: 'values',
      values: [
        { amount: 31.84, unit: { type: 'money', currency: 'USD' } },
        { amount: 796, unit: { type: 'count' } },
      ],
    }
    expect(valuesText(credits)).toMatch(/31\.84 · 796 credits$/)
  })

  it('reads resets as how many are available', () => {
    const resets: Extract<Metric, { kind: 'values' }> = {
      id: 'resets',
      label: 'Rate limit resets',
      kind: 'values',
      values: [{ amount: 3, unit: { type: 'count' } }],
      expiresAt: [],
    }
    expect(valuesText(resets)).toBe('3 available')
  })

  it('compacts large counts', () => {
    expect(formatCompact(1_234_567)).toBe('1.2M')
    expect(formatCompact(999)).toBe('999')
  })
})
