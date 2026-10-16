import type { TrayStatus } from '@/lib/types'

import { useEffect, useState } from 'react'

import { listen } from '@tauri-apps/api/event'

import { getTrayStatus } from '@/lib/commands'

/**
 * Whether the menu bar's fetch is running and when the next is due, kept
 * current by the backend's `tray-status` events.
 */
export function useTrayStatus(): TrayStatus {
  const [status, setStatus] = useState<TrayStatus>({ refreshing: false, nextRefreshAt: null })

  useEffect(() => {
    let active = true
    const unlistenPromise = listen<TrayStatus>('tray-status', (event) => {
      setStatus(event.payload)
    })
    void getTrayStatus().then((initial) => {
      if (active) {
        setStatus(initial)
      }
    })
    return () => {
      active = false
      void unlistenPromise.then((unlisten) => {
        unlisten()
      })
    }
  }, [])

  return status
}
