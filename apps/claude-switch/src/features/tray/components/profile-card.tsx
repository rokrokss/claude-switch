import type { AppId } from '@/lib/app-registry'
import type { Settings, TrayEntry } from '@/lib/types'
import type { Placed } from '../lib/layout'

import { AlertTriangle, ChevronDown, LoaderCircle } from 'lucide-react'

import { cn } from '@/design'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from '@/design/ui/context-menu'
import { BrandMark } from '@/features/profiles/components/brand-mark'
import {
  formatUpdatedAgo,
  quotaErrorMessage,
  quotaErrorShort,
} from '@/features/profiles/components/profile-detail-usage-card'
import { claimCodexReset, openExternalUrl, openSettings, refreshUsage } from '@/lib/commands'

import { cardLayout } from '../lib/layout'
import { starsFor, toggleStar } from '../lib/stars'
import { MetricRow } from './metric-row'

/**
 * Figures older than this are marked outdated: two missed fetches.
 */
const outdatedAfterMs = 10 * 60_000

const quickLinks: Record<AppId, Array<{ label: string; url: string }>> = {
  claude: [
    { label: 'Status', url: 'https://status.anthropic.com' },
    { label: 'Usage', url: 'https://claude.ai/settings/usage' },
  ],
  codex: [
    { label: 'Status', url: 'https://status.openai.com' },
    { label: 'Usage', url: 'https://chatgpt.com/settings/usage' },
  ],
}

type Props = {
  entry: TrayEntry
  settings: Settings
  refreshing: boolean
  now: number
  /**
   * Whether the profile was taken out of the menu bar.
   */
  hidden: boolean
  onSettingsChange: (patch: Partial<Settings>) => void
  onShownChange: (shown: boolean) => void
  /**
   * Says something briefly at the foot of the panel, e.g. why a star was refused.
   */
  onNotice: (text: string) => void
}

/**
 * A profile's card: its name and plan, its always-visible metrics, and —
 * opened — its on-demand ones and links to its status and usage pages.
 */
export function ProfileCard({
  entry,
  settings,
  refreshing,
  now,
  hidden,
  onSettingsChange,
  onShownChange,
  onNotice,
}: Props) {
  const layout = cardLayout(entry, settings.layout)
  const stars = starsFor(settings.menuBar, entry.id)
  const compact = settings.display.density === 'compact'
  const expanded = settings.layout.expanded.includes(entry.id)
  const reportsNothing = layout.alwaysVisible.length === 0 && layout.onDemand.length === 0

  function setExpanded(open: boolean) {
    const others = settings.layout.expanded.filter((id) => id !== entry.id)
    onSettingsChange({ layout: { ...settings.layout, expanded: open ? [...others, entry.id] : others } })
  }

  function rows(placed: Array<Placed>) {
    return placed.map((slot) => (
      <MetricRow
        key={slot.id}
        placed={slot}
        display={settings.display}
        now={now}
        compact={compact}
        starred={!hidden && stars.includes(slot.id)}
        onDisplayChange={(display) => onSettingsChange({ display })}
        onUseReset={
          entry.app === 'codex'
            ? (expiresAt) => claimCodexReset(entry.id, Math.floor(Date.parse(expiresAt) / 1000))
            : undefined
        }
        onNotice={onNotice}
        onStarToggle={() => {
          const result = toggleStar(settings.menuBar, entry.id, slot.id)
          if ('refused' in result) {
            onNotice(result.refused)
            return
          }
          onSettingsChange({ menuBar: result.menuBar })
          if (hidden) {
            onShownChange(true)
          }
        }}
      />
    ))
  }

  return (
    <section
      className={cn(
        'rounded-[10px] border border-border-soft bg-white/40 dark:bg-white/[0.03]',
        compact ? 'px-2.5 py-1.5' : 'px-3 py-2',
      )}
    >
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div>
            <CardHeader
              entry={entry}
              compact={compact}
              refreshing={refreshing}
              now={now}
              showError={!reportsNothing}
              reduceAnimations={settings.display.reduceAnimations}
            />
          </div>
        </ContextMenuTrigger>
        <ContextMenuContent className="w-52 text-[12px]">
          <ContextMenuItem onSelect={() => onShownChange(hidden)}>
            {hidden ? 'Show in menu bar' : 'Hide from menu bar'}
          </ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem onSelect={() => void refreshUsage()}>Update now</ContextMenuItem>
          <ContextMenuItem onSelect={() => void openSettings()}>Settings…</ContextMenuItem>
        </ContextMenuContent>
      </ContextMenu>
      {reportsNothing ? (
        <p className="py-1 font-mono text-mono text-muted-strong">
          {quotaErrorMessage(entry.app, entry.error ?? 'unknown', entry.command)}
        </p>
      ) : (
        rows(layout.alwaysVisible)
      )}
      {expanded ? (
        <div className="mt-1 border-t border-border-soft pt-1">
          {rows(layout.onDemand)}
          <QuickLinks app={entry.app} />
        </div>
      ) : null}
      <button
        type="button"
        aria-expanded={expanded}
        aria-label={expanded ? 'Show less' : 'Show more'}
        className="mt-0.5 flex w-full cursor-pointer justify-center rounded text-muted-strong hover:bg-ink/[0.05] hover:text-ink"
        onClick={() => setExpanded(!expanded)}
      >
        <ChevronDown aria-hidden size={14} className={cn(expanded && 'rotate-180')} />
      </button>
    </section>
  )
}

type HeaderProps = {
  entry: TrayEntry
  compact: boolean
  refreshing: boolean
  now: number
  /**
   * Whether a failed refresh shows as a warning sign; not when the card
   * already says why it has nothing to show.
   */
  showError: boolean
  reduceAnimations: boolean
}

const headerSizes = {
  regular: { gap: 'mb-1', mark: 14, name: 'text-[13px]' },
  compact: { gap: 'mb-0.5', mark: 13, name: 'text-[12.5px]' },
}

/**
 * The card's name line: the app's mark, the profile's name and plan, and
 * whether its figures are being updated, are old, or couldn't be refreshed.
 */
function CardHeader({ entry, compact, ...status }: HeaderProps) {
  const size = headerSizes[compact ? 'compact' : 'regular']
  return (
    <header className={cn('flex items-center justify-between gap-2', size.gap)}>
      <div className="flex min-w-0 items-center gap-1.5">
        <BrandMark app={entry.app} size={size.mark} />
        <span className={cn('truncate font-semibold', size.name)}>{entry.name}</span>
        <span className="shrink-0 text-[11px] text-muted-strong">{entry.plan}</span>
      </div>
      <div className="flex shrink-0 items-center gap-1.5 text-muted-strong">
        <HeaderStatus entry={entry} {...status} />
      </div>
    </header>
  )
}

function HeaderStatus({ entry, refreshing, now, showError, reduceAnimations }: Omit<HeaderProps, 'compact'>) {
  if (refreshing) {
    return <LoaderCircle aria-label="Updating" size={12} className={reduceAnimations ? undefined : 'animate-spin'} />
  }
  return (
    <>
      <OutdatedTag entry={entry} now={now} />
      {showError && entry.error ? (
        <span className="text-amber" title={`Couldn't refresh — ${quotaErrorShort(entry.error)}.`}>
          <AlertTriangle aria-label="Couldn't refresh" size={12} />
        </span>
      ) : null}
    </>
  )
}

function OutdatedTag({ entry, now }: { entry: TrayEntry; now: number }) {
  const fetchedAt = entry.usage ? Date.parse(entry.usage.fetchedAt) : null
  if (fetchedAt === null || now - fetchedAt <= outdatedAfterMs) {
    return null
  }
  return (
    <span
      className="rounded bg-cream-3 px-1.5 py-px font-mono text-[10px]"
      title={`Last updated ${formatUpdatedAgo(now - fetchedAt)}`}
    >
      Outdated
    </span>
  )
}

function QuickLinks({ app }: { app: AppId }) {
  return (
    <div className="mt-1.5 flex gap-1.5">
      {quickLinks[app].map((link) => (
        <button
          key={link.label}
          type="button"
          className="flex-1 cursor-pointer rounded-md border border-border-soft px-2 py-1 text-[11.5px] text-ink-soft hover:bg-ink/[0.05] hover:text-ink"
          onClick={() => {
            void openExternalUrl(link.url)
          }}
        >
          {link.label}
        </button>
      ))}
    </div>
  )
}
