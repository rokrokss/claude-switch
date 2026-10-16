import type { Metric } from '@/lib/types'

import { useState } from 'react'

import { ChevronDown, TriangleAlert } from 'lucide-react'

import { cn } from '@/design'

import { formatCompact, formatCost, formatDay, formatReading, valuesText } from '../lib/format'

type Values = Extract<Metric, { kind: 'values' }>
type Breakdown = NonNullable<Values['breakdown']>
type ModelSpend = Breakdown['models'][number]
type Chart = Extract<Metric, { kind: 'chart' }>

const titleClasses = 'min-w-0 shrink truncate text-[12.5px] font-medium text-ink'
const detailClasses = 'text-[11.5px] tabular-nums text-ink-soft'

/**
 * What a period cost and how many tokens it took, read from this Mac's
 * session logs; it opens to what each model accounts for. A warning mark
 * when a model used then has no price.
 */
export function SpendRow({ metric, spacing }: { metric: Values; spacing: string }) {
  const [open, setOpen] = useState(false)
  const breakdown = metric.breakdown
  const unpriced = (breakdown?.unknownModels?.length ?? 0) > 0
  return (
    <div className={spacing}>
      <button
        type="button"
        aria-expanded={open}
        className="flex w-full cursor-pointer items-center justify-between gap-2 text-left"
        onClick={() => setOpen(!open)}
      >
        <span className={titleClasses}>{metric.label}</span>
        <span className={cn('flex shrink-0 items-center gap-1', detailClasses)}>
          {unpriced ? <TriangleAlert aria-label="A model has no price" size={11} className="text-amber" /> : null}
          {valuesText(metric)}
          <ChevronDown aria-hidden size={12} className={cn('text-muted-strong', open && 'rotate-180')} />
        </span>
      </button>
      {open && breakdown ? <SpendDetails breakdown={breakdown} /> : null}
    </div>
  )
}

function SpendDetails({ breakdown }: { breakdown: Breakdown }) {
  const unknown = breakdown.unknownModels ?? []
  const fallback = breakdown.fallbackModels ?? []
  return (
    <div className="mt-1 rounded-md bg-ink/[0.03] px-2 py-1.5 text-[11.5px] text-ink-soft">
      <ul className="space-y-0.5">
        {breakdown.models.map((model) => (
          <ModelLine key={model.model} model={model} />
        ))}
      </ul>
      <div className="mt-1.5 space-y-0.5 text-muted-strong">
        {unknown.length > 0 ? (
          <p className="text-amber">No price for {unknown.join(', ')}, so it isn't counted.</p>
        ) : null}
        {fallback.length > 0 ? <p>Partly priced at {fallback.join(', ')} rates.</p> : null}
        <p>Estimated at API rates from this Mac's session logs.</p>
      </div>
    </div>
  )
}

/**
 * One model's share; `Other` names the models folded into it.
 */
function ModelLine({ model }: { model: ModelSpend }) {
  const folded = model.variants ?? []
  return (
    <li>
      <div className="flex items-baseline justify-between gap-2">
        <span className="min-w-0 truncate" title={model.model}>
          {model.model}
        </span>
        <span className="shrink-0 tabular-nums">
          {formatCost(model.cost)} · {formatCompact(model.tokens)}
        </span>
      </div>
      {folded.length > 0 ? (
        <div className="truncate text-muted-strong">{folded.map((variant) => variant.model).join(', ')}</div>
      ) : null}
    </li>
  )
}

/**
 * Width of one day's slot in the chart's own units, and the gap between bars.
 */
const step = 10
const gap = 2
const height = 28

/**
 * A reading a day, oldest first, as bars; pointing at one reads its day,
 * otherwise the row reads today.
 */
export function TrendRow({ metric, spacing, compact }: { metric: Chart; spacing: string; compact: boolean }) {
  const [pointed, setPointed] = useState<number | null>(null)
  const { points, unit } = metric
  const last = points.length - 1
  const peak = Math.max(0, ...points.map((point) => point.value))
  const shown = points[pointed ?? last]
  const busiest = points.find((point) => point.value === peak)
  return (
    <div className={spacing}>
      <div className="flex items-baseline justify-between gap-2">
        <span className={titleClasses}>{metric.label}</span>
        {shown ? (
          <span className={detailClasses}>
            {pointed === null || pointed === last ? 'Today' : formatDay(shown.date)} ·{' '}
            {formatReading(shown.value, unit)}
          </span>
        ) : null}
      </div>
      <svg
        role="img"
        aria-label={
          busiest && peak > 0
            ? `Last ${points.length} days; the most on ${formatDay(busiest.date)}: ${formatReading(peak, unit)}`
            : `Last ${points.length} days`
        }
        viewBox={`0 0 ${points.length * step} ${height}`}
        preserveAspectRatio="none"
        className={cn('mt-1 w-full', compact ? 'h-3.5' : 'h-[18px]')}
        onMouseLeave={() => setPointed(null)}
      >
        {points.map((point, index) => {
          const barHeight = peak > 0 && point.value > 0 ? Math.max((point.value / peak) * height, 1.5) : 0
          return (
            // biome-ignore lint/a11y/noStaticElementInteractions: pointing only previews a day the label also gives
            <g key={point.date} data-day={point.date} onMouseEnter={() => setPointed(index)}>
              <rect x={index * step} y={0} width={step} height={height} fill="transparent" />
              <rect
                x={index * step + gap / 2}
                y={height - barHeight}
                width={step - gap}
                height={barHeight}
                rx={1}
                className={barTone(index, pointed, last)}
              />
            </g>
          )
        })}
      </svg>
    </div>
  )
}

function barTone(index: number, pointed: number | null, last: number): string {
  if (index === pointed) {
    return 'fill-ink'
  }
  return index === last ? 'fill-ink-soft' : 'fill-muted-strong/60'
}
