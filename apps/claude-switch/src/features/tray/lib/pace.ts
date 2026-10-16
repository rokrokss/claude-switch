import type { Metric, Settings } from '@/lib/types'

type Meter = Extract<Metric, { kind: 'meter' }>
type MeterState = NonNullable<Meter['state']>
export type Severity = 'normal' | 'warning' | 'critical'

/**
 * How alarming a meter's state is: what colours its bar. `null` without a
 * state to go on.
 */
export function severityOf(state: MeterState | undefined): Severity | null {
  if (!state) {
    return null
  }
  switch (state.kind) {
    case 'spent':
    case 'runningOut':
      return 'critical'
    case 'closeToLimit':
      return 'warning'
    case 'healthy':
      return 'normal'
    case 'level':
      return state.severity
  }
}

/**
 * Where the use projected at the reset lands, said in the one figure the
 * row doesn't already show. `null` for a plain level.
 */
export function projectionNote(state: MeterState | undefined): string | null {
  if (!state) {
    return null
  }
  switch (state.kind) {
    case 'spent':
      return 'Limit reached'
    case 'healthy':
      return `~${Math.round((1 - state.projectedFraction) * 100)}% left at reset`
    case 'closeToLimit':
      return `~${Math.round(state.projectedFraction * 100)}% used at reset`
    case 'runningOut':
      if (state.projectedFraction <= 1) {
        return '~100% used at reset'
      }
      return `~${Math.max(1, Math.round((state.projectedFraction - 1) * 100))}% over limit at reset`
    case 'level':
      return null
  }
}

/**
 * Where on the bar an even pace would be by now, 0 to 1 in the meter's shown
 * direction; `null` where no tick is drawn. Cutting it close and running out
 * always draw it; on course only with "always show pacing".
 */
export function paceTick(
  meter: Meter,
  display: Pick<Settings['display'], 'alwaysShowPacing' | 'meterStyle'>,
  now: number,
): number | null {
  const state = meter.state
  if (!state || state.kind === 'spent' || state.kind === 'level') {
    return null
  }
  if (state.kind === 'healthy' && !display.alwaysShowPacing) {
    return null
  }
  if (!meter.resetsAt || !meter.periodSecs) {
    return null
  }
  const resetsAt = Date.parse(meter.resetsAt)
  const period = meter.periodSecs * 1000
  const elapsed = now - (resetsAt - period)
  if (Number.isNaN(resetsAt) || elapsed < Math.max(60_000, period * 0.01) || now >= resetsAt) {
    return null
  }
  const fraction = Math.min(Math.max(elapsed / period, 0), 1)
  return display.meterStyle === 'left' ? 1 - fraction : fraction
}
