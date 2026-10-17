import { useEffect, useRef, useState } from 'react'

import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window'
import { X } from 'lucide-react'

import { Button, ThemeProvider } from '@/design'
import { closePanel, openMainWindow, openSettings, refreshUsage } from '@/lib/commands'

import { usePanelSettings } from '../api/use-panel-settings'
import { useTrayEntries } from '../api/use-tray-entries'
import { useTrayStatus } from '../api/use-tray-status'
import { useUpdateNotice } from '../api/use-update-notice'
import { hasSpend } from '../lib/total-spend'
import { PanelFooter } from './panel-footer'
import { PartyContext, usePartyMode } from './party'
import { ProfileCard } from './profile-card'
import { TotalSpendCard } from './total-spend-card'

/**
 * The panel is never taller than this share of the screen; past it, it scrolls.
 */
const maxScreenShare = 0.85
const minHeight = 160
const hintKey = 'claude-switch-tray-hint-dismissed'

/**
 * The menu bar panel: every profile's usage, card by card, with the
 * footer's next update and Options menu.
 */
export function TrayPanel() {
  const entries = useTrayEntries()
  const status = useTrayStatus()
  const { settings, themeMode, hiddenFromMenuBar, update } = usePanelSettings()
  const [notice, showNotice] = useNotice()
  const party = usePartyMode()
  const updateVersion = useUpdateNotice()
  const now = useNow()
  const contentRef = useFitWindowToContent()
  usePanelKeys()

  return (
    <ThemeProvider mode={themeMode}>
      <PartyContext.Provider value={party}>
        <div className="h-full overflow-y-auto text-ink">
          <div ref={contentRef} className="flex flex-col gap-2 p-2.5">
            {updateVersion ? (
              <Banner
                text={`claude-switch ${updateVersion} is ready to install.`}
                action="Install…"
                onAction={() => void openMainWindow()}
              />
            ) : null}
            <FirstRunHint />
            {entries === null ? <Note>Loading usage…</Note> : null}
            {entries?.length === 0 ? (
              <div className="flex flex-col items-start gap-2 px-1 py-2">
                <Note>No profile reports usage yet. Set up a profile's CLI and sign in to see it here.</Note>
                <Button size="sm" variant="secondary" onClick={() => void openMainWindow()}>
                  Open claude-switch
                </Button>
              </div>
            ) : null}
            {entries && settings.display.totalSpend.shown && hasSpend(entries) ? (
              <TotalSpendCard
                entries={entries}
                settings={settings.display.totalSpend}
                onChange={(totalSpend) => update({ display: { ...settings.display, totalSpend } })}
                onNotice={showNotice}
              />
            ) : null}
            {entries?.map((entry) => (
              <ProfileCard
                key={entry.id}
                entry={entry}
                settings={settings}
                refreshing={status.refreshing}
                now={now}
                hidden={hiddenFromMenuBar.includes(entry.id)}
                onSettingsChange={update}
                onNotice={showNotice}
              />
            ))}
            {notice ? (
              <p role="status" className="rounded-md bg-amber/15 px-2 py-1 text-center text-[11.5px] text-ink-soft">
                {notice}
              </p>
            ) : null}
            <PanelFooter status={status} now={now} />
          </div>
        </div>
      </PartyContext.Provider>
    </ThemeProvider>
  )
}

function Note({ children }: { children: string }) {
  return <p className="px-1 py-1.5 font-mono text-mono text-muted-strong">{children}</p>
}

function Banner({ text, action, onAction }: { text: string; action: string; onAction: () => void }) {
  return (
    <div className="flex items-center justify-between gap-2 rounded-[10px] border border-border-soft bg-mint/20 px-3 py-2 text-[12px]">
      <span>{text}</span>
      <Button size="sm" variant="secondary" onClick={onAction}>
        {action}
      </Button>
    </div>
  )
}

/**
 * Says once what the panel is and where to change it, until dismissed.
 */
function FirstRunHint() {
  const [dismissed, setDismissed] = useState(() => {
    try {
      return window.localStorage.getItem(hintKey) !== null
    } catch {
      return false
    }
  })
  if (dismissed) {
    return null
  }
  function dismiss() {
    setDismissed(true)
    try {
      window.localStorage.setItem(hintKey, new Date().toISOString())
    } catch {
      // Without storage it comes back next launch; nothing else depends on it.
    }
  }
  return (
    <div className="glass relative rounded-[10px] border px-3 py-2 pr-7 text-[12px]">
      <p className="font-medium">Every profile's usage, at a glance</p>
      <p className="mt-0.5 text-muted-strong">
        Choose what the menu bar shows and what each card lists in{' '}
        <button type="button" className="cursor-pointer underline hover:text-ink" onClick={() => void openSettings()}>
          Settings
        </button>
        .
      </p>
      <button
        type="button"
        aria-label="Dismiss"
        className="absolute top-1.5 right-1.5 cursor-pointer rounded p-0.5 text-muted-strong hover:bg-ink/[0.06] hover:text-ink"
        onClick={dismiss}
      >
        <X aria-hidden size={12} />
      </button>
    </div>
  )
}

/**
 * A short message that clears itself after a few seconds.
 */
function useNotice(): [string | null, (text: string) => void] {
  const [notice, setNotice] = useState<string | null>(null)
  useEffect(() => {
    if (notice === null) {
      return
    }
    const id = setTimeout(() => setNotice(null), 3_000)
    return () => clearTimeout(id)
  }, [notice])
  return [notice, setNotice]
}

/**
 * The time, ticking every few seconds so countdowns and "outdated" stay true.
 */
function useNow(): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 5_000)
    return () => clearInterval(id)
  }, [])
  return now
}

/**
 * Esc closes the panel, ⌘R updates now, ⌘, (or Return) opens Settings.
 */
function usePanelKeys() {
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      const action = keyAction(event)
      if (action) {
        event.preventDefault()
        void action()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
}

/**
 * What a key press in the panel does, if anything. Keys typed into a field
 * or an open menu are theirs.
 */
export function keyAction(event: KeyboardEvent): (() => Promise<void>) | null {
  const target = event.target instanceof Element ? event.target : null
  if (target?.closest('input, textarea, [role="menu"]')) {
    return null
  }
  const onPage = target === null || target === document.body
  if (event.key === 'Escape') {
    return closePanel
  }
  if (event.metaKey && event.key === 'r') {
    return refreshUsage
  }
  if ((event.metaKey && event.key === ',') || (event.key === 'Enter' && onPage)) {
    return openSettings
  }
  return null
}

/**
 * Keeps the window as tall as the element the returned ref is attached to,
 * within limits; past the top one, the panel scrolls.
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
      const max = window.screen.availHeight * maxScreenShare
      const height = Math.max(minHeight, Math.min(element.offsetHeight, max))
      void appWindow.setSize(new LogicalSize(window.innerWidth, Math.ceil(height)))
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  return ref
}
