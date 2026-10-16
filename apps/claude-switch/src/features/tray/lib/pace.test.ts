import type { Metric } from '@/lib/types'

import { describe, expect, it } from 'vitest'

import { paceTick, projectionNote, severityOf } from './pace'

type Meter = Extract<Metric, { kind: 'meter' }>

const hour = 3_600_000
const now = Date.parse('2026-10-17T12:00:00Z')

function meter(state: Meter['state'], overrides: Partial<Meter> = {}): Meter {
  return {
    id: 'session',
    label: 'Session',
    kind: 'meter',
    used: 50,
    limit: 100,
    unit: { type: 'percent' },
    // Halfway through a five-hour window.
    resetsAt: new Date(now + 2.5 * hour).toISOString(),
    periodSecs: 5 * 3600,
    notStarted: false,
    state,
    ...overrides,
  }
}

const used = { alwaysShowPacing: false, meterStyle: 'used' as const }

describe('severityOf', () => {
  it('colours by how alarming the state is', () => {
    expect(severityOf({ kind: 'spent' })).toBe('critical')
    expect(severityOf({ kind: 'closeToLimit', sparePercent: 6, projectedFraction: 0.94 })).toBe('warning')
    expect(severityOf({ kind: 'healthy', projectedFraction: 0.4 })).toBe('normal')
    expect(severityOf({ kind: 'level', severity: 'warning' })).toBe('warning')
    expect(severityOf(undefined)).toBeNull()
  })
})

describe('projectionNote', () => {
  it('says where pace lands at the reset', () => {
    expect(projectionNote({ kind: 'healthy', projectedFraction: 0.65 })).toBe('~35% left at reset')
    expect(projectionNote({ kind: 'closeToLimit', sparePercent: 8, projectedFraction: 0.92 })).toBe(
      '~92% used at reset',
    )
    expect(projectionNote({ kind: 'runningOut', runsOutAt: null, projectedFraction: 1.12 })).toBe(
      '~12% over limit at reset',
    )
    expect(projectionNote({ kind: 'runningOut', runsOutAt: null, projectedFraction: 0.998 })).toBe(
      '~100% used at reset',
    )
    expect(projectionNote({ kind: 'level', severity: 'normal' })).toBeNull()
  })
})

describe('paceTick', () => {
  it('marks the even pace when cutting it close, in the shown direction', () => {
    const close = meter({ kind: 'closeToLimit', sparePercent: 6, projectedFraction: 0.94 })
    expect(paceTick(close, used, now)).toBeCloseTo(0.5)
    expect(paceTick(close, { ...used, meterStyle: 'left' }, now)).toBeCloseTo(0.5)
    const early = meter(
      { kind: 'closeToLimit', sparePercent: 6, projectedFraction: 0.94 },
      { resetsAt: new Date(now + 4 * hour).toISOString() },
    )
    expect(paceTick(early, { ...used, meterStyle: 'left' }, now)).toBeCloseTo(0.8)
  })

  it('marks an on-course meter only when asked to', () => {
    const healthy = meter({ kind: 'healthy', projectedFraction: 0.4 })
    expect(paceTick(healthy, used, now)).toBeNull()
    expect(paceTick(healthy, { ...used, alwaysShowPacing: true }, now)).toBeCloseTo(0.5)
  })

  it('draws no tick without a window, or once it has reset', () => {
    expect(paceTick(meter({ kind: 'level', severity: 'normal' }), used, now)).toBeNull()
    const past = meter(
      { kind: 'runningOut', runsOutAt: null, projectedFraction: 1.2 },
      { resetsAt: new Date(now - hour).toISOString() },
    )
    expect(paceTick(past, used, now)).toBeNull()
  })
})
