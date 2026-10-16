import type { TrayEntry } from '@/lib/types'

import { useEffect, useState } from 'react'

import { listen } from '@tauri-apps/api/event'

import { getTrayUsage } from '@/lib/commands'

/**
 * The profiles the menu bar reports on. The backend fetches their usage on
 * its own schedule and sends the entries whenever they change, and again each
 * time the panel opens. `null` until the first fetch has finished.
 */
export function useTrayEntries(): Array<TrayEntry> | null {
  const [entries, setEntries] = useState<Array<TrayEntry> | null>(null)

  useEffect(() => {
    let active = true
    const unlistenPromise = listen<Array<TrayEntry>>('tray-usage', (event) => {
      setEntries(event.payload)
    })
    void getTrayUsage().then((initial) => {
      // An event that arrived first is at least as new.
      if (active && initial !== null) {
        setEntries((current) => current ?? initial)
      }
    })
    return () => {
      active = false
      void unlistenPromise.then((unlisten) => {
        unlisten()
      })
    }
  }, [])

  return entries
}
