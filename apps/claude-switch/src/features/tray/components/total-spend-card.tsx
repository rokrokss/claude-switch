import type { SegmentedOption } from '@/design'
import type { Settings, TrayEntry } from '@/lib/types'
import type { SpendSlice, TotalSpend } from '../lib/total-spend'

import { cn, Segmented } from '@/design'

import { formatCompact, formatCost } from '../lib/format'
import { costPerMillion, shareOf, totalSpend } from '../lib/total-spend'

type TotalSpendSettings = Settings['display']['totalSpend']
type Period = TotalSpendSettings['period']
type Shown = TotalSpendSettings['metric']

const periodOptions: ReadonlyArray<SegmentedOption<Period>> = [
  { value: 'today', label: 'Today' },
  { value: 'yesterday', label: 'Yesterday' },
  { value: 'last30Days', label: '30 days' },
]

const shownOptions: ReadonlyArray<SegmentedOption<Shown>> = [
  { value: 'cost', label: 'Cost' },
  { value: 'costPerMillionTokens', label: '$/M', ariaLabel: 'Cost per million tokens' },
  { value: 'tokens', label: 'Tokens' },
]

const nothingIn: Record<Period, string> = {
  today: 'Nothing spent today yet.',
  yesterday: 'Nothing spent yesterday.',
  last30Days: 'Nothing spent in the last 30 days.',
}

/**
 * Literal class names, so the stylesheet keeps them.
 */
const seriesTone = ['bg-series-1', 'bg-series-2', 'bg-series-3', 'bg-series-4', 'bg-series-5', 'bg-series-6']

type Props = {
  entries: Array<TrayEntry>
  settings: TotalSpendSettings
  onChange: (settings: TotalSpendSettings) => void
}

/**
 * What every profile spent in a period, read from this Mac's session logs:
 * the total, a bar of each profile's share, and a legend with the figures.
 */
export function TotalSpendCard({ entries, settings, onChange }: Props) {
  const total = totalSpend(entries, settings.period)
  return (
    <section
      aria-label="Total spend"
      className="rounded-[10px] border border-border-soft bg-white/40 px-3 py-2 dark:bg-white/[0.03]"
    >
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-[12.5px] font-medium text-ink">Total spend</h2>
        <Segmented<Period>
          ariaLabel="Period"
          size="small"
          options={periodOptions}
          value={settings.period}
          onChange={(period) => onChange({ ...settings, period })}
        />
      </div>
      {total.slices.length === 0 ? (
        <p className="mt-1.5 text-[11.5px] text-muted-strong">{nothingIn[settings.period]}</p>
      ) : (
        <>
          <div className="mt-1.5 flex items-center justify-between gap-2">
            <span className="text-[19px] font-semibold tabular-nums text-ink">
              {headline(total.cost, total.tokens, settings.metric)}
            </span>
            <Segmented<Shown>
              ariaLabel="Show"
              size="small"
              options={shownOptions}
              value={settings.metric}
              onChange={(metric) => onChange({ ...settings, metric })}
            />
          </div>
          <ShareBar total={total} shown={settings.metric} />
          <ul className="mt-1.5 space-y-0.5">
            {total.slices.map((slice) => (
              <LegendLine
                key={slice.id}
                slice={slice}
                share={shareOf(slice, total, settings.metric)}
                shown={settings.metric}
              />
            ))}
          </ul>
        </>
      )}
    </section>
  )
}

/**
 * The whole, or a slice, as `shown` reads it: `$48.20`, `$3.10/M`, `12.4M tokens`.
 */
function headline(cost: number, tokens: number, shown: Shown): string {
  if (shown === 'tokens') {
    return `${formatCompact(tokens)} tokens`
  }
  if (shown === 'costPerMillionTokens') {
    const perMillion = costPerMillion(cost, tokens)
    return perMillion === null ? '—' : `${formatCost(perMillion)}/M`
  }
  return formatCost(cost)
}

function toneOf(slot: number | null): string {
  return slot === null ? 'bg-muted-strong' : (seriesTone[slot - 1] ?? 'bg-muted-strong')
}

function percent(share: number): string {
  return `${Math.round(share * 100)}%`
}

/**
 * Each profile's share of tokens, when tokens are shown, otherwise of cost.
 */
function ShareBar({ total, shown }: { total: TotalSpend; shown: Shown }) {
  const shares = total.slices
    .map((slice) => ({ slice, share: shareOf(slice, total, shown) }))
    .filter(({ share }) => share > 0)
  const of = shown === 'tokens' ? 'tokens' : 'cost'
  return (
    <div
      role="img"
      aria-label={`Share of ${of}: ${shares.map(({ slice, share }) => `${slice.name} ${percent(share)}`).join(', ')}`}
      className="mt-1.5 flex h-2 gap-[2px] overflow-hidden rounded-full bg-cream-3"
    >
      {shares.map(({ slice, share }) => (
        <div
          key={slice.id}
          title={`${slice.name}: ${percent(share)} of ${of}`}
          className={cn('h-full min-w-[3px]', toneOf(slice.slot))}
          style={{ flexGrow: share, flexBasis: 0 }}
        />
      ))}
    </div>
  )
}

function LegendLine({ slice, share, shown }: { slice: SpendSlice; share: number; shown: Shown }) {
  return (
    <li className="flex items-center gap-1.5 text-[11.5px]">
      <span aria-hidden className={cn('h-2 w-2 shrink-0 rounded-full', toneOf(slice.slot))} />
      <span className="min-w-0 flex-1 truncate text-ink-soft">{slice.name}</span>
      <span className="tabular-nums text-ink">{headline(slice.cost, slice.tokens, shown)}</span>
      <span className="w-8 shrink-0 text-right tabular-nums text-muted-strong">{percent(share)}</span>
    </li>
  )
}
