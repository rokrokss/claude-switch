import type { Metric, ResetOutcome, Settings } from '@/lib/types'
import type { Placed } from '../lib/layout'
import type { Severity } from '../lib/pace'

import { Flame, Star } from 'lucide-react'

import { cn, TooltipBubble } from '@/design'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from '@/design/ui/context-menu'
import { openSettings, refreshUsage } from '@/lib/commands'

import { formatDeadline, formatDuration, meterHeadline, shownFraction, valuesText } from '../lib/format'
import { paceTick, projectionNote, severityOf } from '../lib/pace'
import { canStar } from '../lib/stars'
import { useParty } from './party'
import { ResetsRow } from './resets-row'
import { SpendRow, TrendRow } from './spend-rows'

type Meter = Extract<Metric, { kind: 'meter' }>
type Display = Settings['display']

type Props = {
  placed: Placed
  display: Display
  now: number
  compact: boolean
  /**
   * Whether the metric is starred for the menu bar.
   */
  starred: boolean
  onDisplayChange: (display: Display) => void
  onStarToggle: () => void
  /**
   * Uses a rate-limit reset; given where the profile's resets can be used.
   */
  onUseReset?: (expiresAt: string) => Promise<ResetOutcome>
  onNotice: (text: string) => void
}

const barTone: Record<Severity, string> = {
  normal: 'bg-green',
  warning: 'bg-amber',
  critical: 'bg-red',
}

const textTone: Record<Severity, string> = {
  normal: 'text-muted-strong',
  warning: 'text-amber',
  critical: 'text-red',
}

const titleClasses = 'min-w-0 shrink truncate text-[12.5px] font-medium text-ink'
const detailClasses = 'text-[11.5px] tabular-nums text-muted-strong'
const toggleClasses = 'cursor-pointer rounded px-0.5 -mx-0.5 hover:bg-ink/[0.06] hover:text-ink'

/**
 * One metric of a profile's card, with its right-click menu: stars it for
 * the menu bar, updates, or opens Settings.
 */
export function MetricRow({ starred, onStarToggle, ...props }: Props) {
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div className="relative">
          {starred ? (
            <Star
              aria-label="In the menu bar"
              size={8}
              className="absolute top-2 -left-2.5 fill-current text-muted-strong"
            />
          ) : null}
          <MetricBody {...props} />
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent className="w-52 text-[12px]">
        <ContextMenuItem disabled={!starred && !canStar(props.placed.id)} onSelect={onStarToggle}>
          {starred ? 'Remove from menu bar' : 'Star for menu bar'}
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={() => void refreshUsage()}>Update now</ContextMenuItem>
        <ContextMenuItem onSelect={() => void openSettings()}>Settings…</ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}

/**
 * The metric itself: a meter, a reading, a day-by-day trend, or — for a
 * metric the user placed that the profile doesn't report — "No data".
 */
function MetricBody({
  placed,
  display,
  now,
  compact,
  onDisplayChange,
  onUseReset,
  onNotice,
}: Omit<Props, 'starred' | 'onStarToggle'>) {
  const spacing = compact ? 'py-0.5' : 'py-1'
  if (placed.metric === null) {
    return (
      <div className={cn('flex items-center justify-between gap-2', spacing)}>
        <span className={titleClasses}>{placed.label}</span>
        <span className={detailClasses}>No data</span>
      </div>
    )
  }
  if (placed.metric.kind === 'values') {
    if (placed.metric.id === 'resets' && onUseReset) {
      return <ResetsRow metric={placed.metric} now={now} spacing={spacing} onUse={onUseReset} onNotice={onNotice} />
    }
    if (placed.metric.breakdown) {
      return <SpendRow metric={placed.metric} spacing={spacing} />
    }
    return <ValuesRow metric={placed.metric} now={now} spacing={spacing} />
  }
  if (placed.metric.kind === 'chart') {
    return <TrendRow metric={placed.metric} spacing={spacing} compact={compact} />
  }
  return (
    <MeterRow
      meter={placed.metric}
      display={display}
      now={now}
      compact={compact}
      spacing={spacing}
      onDisplayChange={onDisplayChange}
    />
  )
}

type MeterProps = {
  meter: Meter
  display: Display
  now: number
  compact: boolean
  onDisplayChange: (display: Display) => void
}

function MeterRow({ spacing, ...props }: MeterProps & { spacing: string }) {
  return (
    <div className={spacing}>
      <MeterTitle {...props} />
      <MeterBar {...props} />
      <MeterDetails {...props} />
    </div>
  )
}

/**
 * The meter's label, and its warning when it isn't simply on course.
 */
function MeterTitle({ meter, display, now }: MeterProps) {
  const severity = severityOf(meter.state)
  const warning = warningText(meter, display, now)
  return (
    <div className="flex items-baseline justify-between gap-2">
      <span className={titleClasses}>{meter.label}</span>
      {warning ? (
        <span className={cn('flex shrink-0 items-center gap-1', detailClasses, severity && textTone[severity])}>
          {warning.flame ? <Flame aria-hidden size={11} /> : null}
          {warning.text}
        </span>
      ) : null}
    </div>
  )
}

/**
 * The bar, coloured by the meter's state, with the even-pace tick and the
 * projection on hover.
 */
function MeterBar({ meter, display, now, compact }: MeterProps) {
  const party = useParty()
  const severity = severityOf(meter.state)
  const fill = shownFraction(meter, display.meterStyle) * 100
  const tick = paceTick(meter, display, now)
  // A spent meter's warning already says it.
  const note = meter.state?.kind === 'spent' ? null : projectionNote(meter.state)
  return (
    <div className="group relative mt-1">
      <div
        role="progressbar"
        aria-label={meter.label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(fill)}
        className={cn('overflow-hidden rounded-full bg-cream-3', compact ? 'h-1' : 'h-[5px]')}
      >
        <div
          className={fillClasses(severity, party, !display.reduceAnimations)}
          style={{ width: `${fill}%`, minWidth: fill > 0 ? 6 : 0 }}
        />
      </div>
      <PaceTick at={tick} compact={compact} />
      {note ? <TooltipBubble>{note}</TooltipBubble> : null}
    </div>
  )
}

/**
 * The bar's fill: coloured by severity, or a rainbow in party mode.
 */
function fillClasses(severity: Severity | null, party: boolean, moving: boolean): string {
  if (party) {
    return cn('h-full rounded-full party-fill', moving && 'party-fill-moving')
  }
  return cn('h-full rounded-full', severity ? barTone[severity] : 'bg-muted-strong')
}

/**
 * Where an even pace would be by now, as a mark across the bar.
 */
function PaceTick({ at, compact }: { at: number | null; compact: boolean }) {
  if (at === null) {
    return null
  }
  return (
    <div
      aria-hidden
      className={cn('pointer-events-none absolute -top-0.5 w-0.5 rounded-sm bg-ink/55', compact ? 'h-2' : 'h-[9px]')}
      style={{ left: `calc(${at * 100}% - 1px)` }}
    />
  )
}

/**
 * The reading (a click shows it the other way, used or left) and when the
 * window resets (a click flips countdown and time).
 */
function MeterDetails({ meter, display, now, onDisplayChange }: MeterProps) {
  const otherStyle = display.meterStyle === 'used' ? 'left' : 'used'
  return (
    <div className={cn('mt-1 flex items-center justify-between gap-2', detailClasses)}>
      <button
        type="button"
        title={`Show ${otherStyle}`}
        className={toggleClasses}
        onClick={() => onDisplayChange({ ...display, meterStyle: otherStyle })}
      >
        {meterHeadline(meter, display.meterStyle)}
      </button>
      <ResetLabel meter={meter} display={display} now={now} onDisplayChange={onDisplayChange} />
    </div>
  )
}

function ResetLabel({ meter, display, now, onDisplayChange }: Omit<MeterProps, 'compact'>) {
  if (meter.notStarted) {
    return <span title="It starts with your first message.">Not started</span>
  }
  if (!meter.resetsAt) {
    return null
  }
  const resetsAt = new Date(meter.resetsAt)
  const other = display.resetDisplay === 'countdown' ? 'exact' : 'countdown'
  return (
    <button
      type="button"
      title={formatDeadline('Resets', resetsAt, other, display.timeFormat, new Date(now))}
      className={toggleClasses}
      onClick={() => onDisplayChange({ ...display, resetDisplay: other })}
    >
      {formatDeadline('Resets', resetsAt, display.resetDisplay, display.timeFormat, new Date(now))}
    </button>
  )
}

/**
 * The note above a meter that isn't simply on course: spent, when it runs
 * out, how much it has to spare — or, with "always show pacing", where an
 * on-course meter lands.
 */
function warningText(meter: Meter, display: Display, now: number): { text: string; flame: boolean } | null {
  const state = meter.state
  switch (state?.kind) {
    case 'spent':
      return { text: 'Limit reached', flame: true }
    case 'runningOut':
      return {
        text: state.runsOutAt
          ? formatDeadline('Limit', new Date(state.runsOutAt), display.resetDisplay, display.timeFormat, new Date(now))
          : '',
        flame: true,
      }
    case 'closeToLimit':
      return { text: `~${state.sparePercent}% spare`, flame: false }
    case 'healthy':
      return display.alwaysShowPacing ? { text: projectionNote(state) ?? '', flame: false } : null
    default:
      return null
  }
}

/**
 * A reading without a limit. Resets carry a dot coloured by how soon the
 * first of them lapses.
 */
function ValuesRow({
  metric,
  now,
  spacing,
}: {
  metric: Extract<Metric, { kind: 'values' }>
  now: number
  spacing: string
}) {
  const soonest = metric.expiresAt?.[0]
  const untilLapse = soonest ? Date.parse(soonest) - now : null
  return (
    <div className={cn('flex items-center justify-between gap-2', spacing)}>
      <span className={titleClasses}>{metric.label}</span>
      <span
        className={cn('flex shrink-0 items-center gap-1.5', detailClasses, 'text-ink-soft')}
        title={untilLapse === null ? undefined : `The first lapses in ${formatDuration(untilLapse)}`}
      >
        {untilLapse === null ? null : (
          <span aria-hidden className={cn('h-1.5 w-1.5 rounded-full', barTone[expirySeverity(untilLapse)])} />
        )}
        {valuesText(metric)}
      </span>
    </div>
  )
}

/**
 * Red within two days, amber within a week.
 */
function expirySeverity(ms: number): Severity {
  const day = 24 * 3_600_000
  if (ms <= 2 * day) {
    return 'critical'
  }
  if (ms <= 7 * day) {
    return 'warning'
  }
  return 'normal'
}
