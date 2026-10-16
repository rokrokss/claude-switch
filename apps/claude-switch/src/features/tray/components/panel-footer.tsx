import type { TrayStatus } from '@/lib/types'

import { useEffect, useState } from 'react'

import { ChevronDown } from 'lucide-react'

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from '@/design/ui/dropdown-menu'
import { getAppMetadata, openAbout, openExternalUrl, openSettings, quitApp, refreshUsage } from '@/lib/commands'

import { formatDuration } from '../lib/format'

const issuesUrl = 'https://github.com/rokrokss/claude-switch/issues/new'

/**
 * The panel's last line: the version and when the next update comes (a
 * click updates now), and the Options menu.
 */
export function PanelFooter({ status, now }: { status: TrayStatus; now: number }) {
  const version = useVersion()
  const next = status.nextRefreshAt ? formatDuration(Date.parse(status.nextRefreshAt) - now) : null
  let schedule = ''
  if (status.refreshing) {
    schedule = 'Updating…'
  } else if (next) {
    schedule = next === 'soon' ? 'Updating soon' : `Next update in ${next}`
  }

  return (
    <footer className="flex items-center justify-between gap-2 px-1 pt-0.5">
      <button
        type="button"
        disabled={status.refreshing}
        title="Update now (⌘R)"
        className="min-w-0 cursor-pointer truncate text-left font-mono text-mono text-muted-strong hover:text-ink disabled:cursor-default"
        onClick={() => {
          void refreshUsage()
        }}
      >
        claude-switch {version}
        {schedule ? ` · ${schedule}` : ''}
      </button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="flex shrink-0 cursor-pointer items-center gap-1 rounded-md border border-border-soft px-2 py-1 text-[12px] text-ink-soft hover:bg-ink/[0.05] hover:text-ink"
          >
            Options
            <ChevronDown aria-hidden size={12} />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-52 text-[12px]">
          <DropdownMenuItem onSelect={() => void openSettings()}>
            Settings…
            <DropdownMenuShortcut>⌘,</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void refreshUsage()}>
            Update now
            <DropdownMenuShortcut>⌘R</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => void openExternalUrl(issuesUrl)}>Report an issue…</DropdownMenuItem>
          <DropdownMenuItem onSelect={() => void openAbout()}>About claude-switch</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => void quitApp()}>
            Quit claude-switch
            <DropdownMenuShortcut>⌘Q</DropdownMenuShortcut>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </footer>
  )
}

function useVersion(): string {
  const [version, setVersion] = useState('')
  useEffect(() => {
    void getAppMetadata().then((metadata) => {
      setVersion(metadata.version)
    })
  }, [])
  return version
}
