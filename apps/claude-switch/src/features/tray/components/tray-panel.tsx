import type { ThemeMode, TrayEntry } from '@/lib/types'

import { useEffect, useRef, useState } from 'react'

import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'

import { Button, ThemeProvider } from '@/design'
import { narrowProfileUsage } from '@/features/profiles/api/narrow-usage'
import { BrandMark } from '@/features/profiles/components/brand-mark'
import {
  formatUpdatedAgo,
  Meters,
  quotaErrorMessage,
  quotaErrorShort,
} from '@/features/profiles/components/profile-detail-usage-card'
import { loadAppState, openMainWindow, quitApp } from '@/lib/commands'

import { useTrayEntries } from '../api/use-tray-entries'

/**
 * Space kept between a panel at its tallest and the bottom of the screen,
 * the menu bar included.
 */
const screenMargin = 64

/**
 * The menu bar panel: every profile's meters, the same ones its detail page
 * shows, with a way back to the main window.
 */
export function TrayPanel() {
  const entries = useTrayEntries()
  const themeMode = usePersistedThemeMode()
  const contentRef = useFitWindowToContent()

  return (
    <ThemeProvider mode={themeMode}>
      <div className="h-full overflow-y-auto bg-base text-ink">
        <div ref={contentRef} className="flex flex-col gap-2.5 p-3">
          {entries === null ? <Note>Loading usage…</Note> : null}
          {entries?.length === 0 ? (
            <Note>No profile reports usage yet. Set up a profile's CLI and sign in to see it here.</Note>
          ) : null}
          {entries?.map((entry) => (
            <TrayProfileCard key={entry.id} entry={entry} />
          ))}
          <footer className="flex items-center justify-end gap-1.5 pt-0.5">
            <Button size="sm" variant="ghost" onClick={() => void quitApp()}>
              Quit
            </Button>
            <Button size="sm" variant="secondary" onClick={() => void openMainWindow()}>
              Open claude-switch
            </Button>
          </footer>
        </div>
      </div>
    </ThemeProvider>
  )
}

function Note({ children }: { children: string }) {
  return <p className="px-1 py-2 font-mono text-mono text-muted-strong">{children}</p>
}

function TrayProfileCard({ entry }: { entry: TrayEntry }) {
  const usage = entry.usage ? narrowProfileUsage(entry.usage) : null
  const quota = usage?.quota ?? null
  return (
    <section className="rounded-[10px] border border-border-soft bg-white/30 px-[13px] py-[9px] dark:bg-white/[0.02]">
      <header className="mb-1.5 flex min-h-[22px] items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-1.5">
          <BrandMark app={entry.app} size={14} />
          <span className="truncate font-medium text-body">{entry.name}</span>
        </div>
        {usage ? (
          <span className="shrink-0 font-mono text-mono text-muted-strong">
            updated {formatUpdatedAgo(Date.now() - new Date(usage.fetchedAt).getTime())}
          </span>
        ) : null}
      </header>
      {quota ? (
        <div className="flex flex-col gap-2">
          <Meters app={entry.app} quota={quota} />
          {entry.error ? (
            <p className="font-mono text-mono text-muted-strong">Couldn't refresh — {quotaErrorShort(entry.error)}.</p>
          ) : null}
        </div>
      ) : (
        <p className="font-mono text-mono text-muted-strong">
          {quotaErrorMessage(entry.app, entry.error ?? 'unknown', entry.command)}
        </p>
      )}
    </section>
  )
}

/**
 * The theme the user picked in Settings. Read again whenever the panel gains
 * focus, which is each time it opens, so a change made in the main window
 * reaches it.
 */
function usePersistedThemeMode(): ThemeMode {
  const [mode, setMode] = useState<ThemeMode>('system')

  useEffect(() => {
    function load() {
      void loadAppState().then((state) => {
        setMode(state.themeMode)
      })
    }
    load()
    const unlistenPromise = getCurrentWindow().onFocusChanged((event) => {
      if (event.payload) {
        load()
      }
    })
    return () => {
      void unlistenPromise.then((unlisten) => {
        unlisten()
      })
    }
  }, [])

  return mode
}

/**
 * Keeps the window as tall as the element the returned ref is attached to,
 * up to what fits on screen; past that, the panel scrolls.
 */
function useFitWindowToContent() {
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const element = ref.current
    if (!element) {
      return
    }
    const appWindow = getCurrentWindow()
    const observer = new ResizeObserver(() => {
      const height = Math.min(element.offsetHeight, window.screen.availHeight - screenMargin)
      void appWindow.setSize(new LogicalSize(window.innerWidth, Math.ceil(height)))
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  return ref
}
