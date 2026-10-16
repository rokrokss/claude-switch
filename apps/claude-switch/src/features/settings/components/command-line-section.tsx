import type { CommandLine } from '@/lib/types'

import { useState } from 'react'

import { Check } from 'lucide-react'

import { Button, cn, Skeleton } from '@/design'
import { useCommandLine } from '@/features/settings/api/use-command-line'
import { useAppState } from '@/lib/app-state/use-app-state'
import { extractErrorMessage } from '@/lib/extract-error-message'

/**
 * Where the local API serves the limits; the port is `PORT` in
 * src-tauri/src/local_api/mod.rs.
 */
const LOCAL_API_URL = 'http://127.0.0.1:6741/v1/limits'

function commandDetail(commandLine: CommandLine): string {
  switch (commandLine.state) {
    case 'installed':
      return `Installed at ${commandLine.path}. Run it in a new terminal to print each profile's limits as JSON.`
    case 'notInstalled':
      return `Prints each profile's limits as JSON, for scripts and agents. Installs to ${commandLine.path}.`
    case 'conflict':
      return `${commandLine.path} belongs to something else, so claude-switch leaves it alone.`
  }
}

/**
 * The Command line section: the `claude-switch usage` command, installed into
 * `~/.local/bin` or removed, and the local API that serves the same figures.
 *
 * Suspends on the command's status; the shell wraps it in its own boundary.
 */
export function CommandLineSection() {
  const { commandLine, install, uninstall, busy } = useCommandLine()
  const appState = useAppState()
  const [error, setError] = useState<string | null>(null)
  const installed = commandLine.state === 'installed'

  async function handleCommand() {
    setError(null)
    try {
      await (installed ? uninstall() : install())
    } catch (caught) {
      setError(extractErrorMessage(caught))
    }
  }

  function handleLocalApi(next: boolean) {
    void appState.update({ settings: { integrations: { localApi: next } } })
  }

  return (
    <section className="mb-8">
      <div className="mb-2.5 flex items-center justify-between">
        <span className="font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-strong">
          Command line
        </span>
      </div>
      <div className="glass rounded-xl border">
        <div className="flex items-start gap-3 border-b border-border-soft p-3">
          <span className="flex-1">
            <span className="block font-mono text-[12.5px] font-medium text-ink">claude-switch usage</span>
            <span className="mt-0.5 block text-[12px] leading-[1.4] text-muted">{commandDetail(commandLine)}</span>
          </span>
          <Button
            size="sm"
            variant="secondary"
            disabled={busy || commandLine.state === 'conflict'}
            onClick={() => void handleCommand()}
          >
            {installed ? 'Uninstall' : 'Install'}
          </Button>
        </div>
        <LocalApiRow checked={appState.state.settings.integrations.localApi} onChange={handleLocalApi} />
      </div>
      {error ? (
        <p role="alert" className="mt-1 text-[11.5px] text-red">
          {error}
        </p>
      ) : null}
    </section>
  )
}

type LocalApiRowProps = {
  /**
   * Whether the local API is on.
   */
  checked: boolean
  onChange: (next: boolean) => void
}

/**
 * The local API's switch, with the note that any web page can read it.
 */
function LocalApiRow({ checked, onChange }: LocalApiRowProps) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: rich row layout with description copy precludes a native <input type="checkbox">
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={cn(
        'flex w-full cursor-pointer items-start gap-3 rounded-b-xl p-3 text-left transition-colors duration-(--duration-snap) ease-(--ease-natural)',
        'hover:bg-black/[0.02] dark:hover:bg-white/[0.03]',
        'focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-focus/40',
      )}
    >
      <span
        aria-hidden
        className={cn(
          'mt-px grid h-4 w-4 shrink-0 place-items-center rounded-[5px] border-[1.5px] transition-colors duration-(--duration-snap) ease-(--ease-natural)',
          checked ? 'border-ink bg-mint' : 'border-border bg-cream',
        )}
      >
        {checked ? <Check className="h-[11px] w-[11px] text-accent-ink" strokeWidth={3} /> : null}
      </span>
      <span className="flex-1">
        <span className="block text-[13px] font-medium text-ink">Local API</span>
        <span className="mt-0.5 block text-[12px] leading-[1.4] text-muted">
          Serves the same figures at <span className="font-mono">{LOCAL_API_URL}</span> for other apps. Any web page
          open on this Mac can read them while it's on.
        </span>
      </span>
    </button>
  )
}

/**
 * Skeleton for the Command line section while the command's status loads.
 */
export function CommandLineSectionFallback() {
  return (
    <section className="mb-8">
      <div className="mb-2.5 flex items-center justify-between">
        <span className="font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-strong">
          Command line
        </span>
      </div>
      <Skeleton className="h-[132px] w-full rounded-xl" />
    </section>
  )
}
