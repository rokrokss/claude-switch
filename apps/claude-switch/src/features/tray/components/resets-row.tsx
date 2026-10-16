import type { Metric, ResetOutcome } from '@/lib/types'

import { useState } from 'react'

import { ChevronDown } from 'lucide-react'

import { Button, cn } from '@/design'

import { formatDuration, valuesText } from '../lib/format'

const outcomeNotice: Record<ResetOutcome, string> = {
  reset: 'Reset used: your Codex limits start over.',
  nothingToReset: 'Nothing to reset yet: there is no usage to clear.',
  noCredit: 'That reset is gone: spent or lapsed meanwhile.',
  failed: "Couldn't use the reset. Try again in a moment.",
}

type Props = {
  metric: Extract<Metric, { kind: 'values' }>
  now: number
  spacing: string
  /**
   * Uses the reset lapsing then; absent where resets can't be used here.
   */
  onUse: (expiresAt: string) => Promise<ResetOutcome>
  onNotice: (text: string) => void
}

/**
 * Codex's rate-limit resets: how many, opening to each one's expiry and a
 * way to use it, after a confirmation since it can't be undone.
 */
export function ResetsRow({ metric, now, spacing, onUse, onNotice }: Props) {
  const [open, setOpen] = useState(false)
  const [confirming, setConfirming] = useState<string | null>(null)
  const [using, setUsing] = useState(false)
  const expiries = metric.expiresAt ?? []

  async function use(expiresAt: string) {
    setUsing(true)
    try {
      onNotice(outcomeNotice[await onUse(expiresAt)])
    } catch {
      onNotice(outcomeNotice.failed)
    } finally {
      setUsing(false)
      setConfirming(null)
    }
  }

  return (
    <div className={spacing}>
      <button
        type="button"
        aria-expanded={open}
        className="flex w-full cursor-pointer items-center justify-between gap-2 text-left"
        onClick={() => setOpen(!open)}
      >
        <span className="min-w-0 shrink truncate text-[12.5px] font-medium text-ink">{metric.label}</span>
        <span className="flex shrink-0 items-center gap-1 text-[11.5px] tabular-nums text-ink-soft">
          {valuesText(metric)}
          <ChevronDown aria-hidden size={12} className={cn('text-muted-strong', open && 'rotate-180')} />
        </span>
      </button>
      {open ? (
        <ul className="mt-1 space-y-1">
          {expiries.length === 0 ? <li className="text-[11.5px] text-muted-strong">No expiry details.</li> : null}
          {expiries.map((expiresAt, index) => (
            <li key={expiresAt} className="rounded-md bg-ink/[0.03] px-2 py-1 text-[11.5px] text-ink-soft">
              {confirming === expiresAt ? (
                <div className="flex items-center justify-between gap-2">
                  <span>Reset your Codex limits now? It can't be undone.</span>
                  <span className="flex shrink-0 gap-1">
                    <Button size="sm" variant="ghost" disabled={using} onClick={() => setConfirming(null)}>
                      Cancel
                    </Button>
                    <Button size="sm" variant="primary" disabled={using} onClick={() => void use(expiresAt)}>
                      Reset
                    </Button>
                  </span>
                </div>
              ) : (
                <div className="flex items-center justify-between gap-2">
                  <span className="tabular-nums">
                    #{index + 1} · lapses in {formatDuration(Date.parse(expiresAt) - now)}
                  </span>
                  <Button size="sm" variant="secondary" onClick={() => setConfirming(expiresAt)}>
                    Use
                  </Button>
                </div>
              )}
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  )
}
