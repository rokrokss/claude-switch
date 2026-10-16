// SPDX-License-Identifier: MIT
import { useEffect, useRef } from 'react'

import { emitTo } from '@tauri-apps/api/event'

import { useToast } from '@/design'

import { useUpdater } from '../api/use-updater'

/**
 * Mounts at the app root. Renders nothing — its only job is to convert
 * the updater's `available` state into a side-effect toast that prompts
 * the user to restart and install. The ref guard suppresses re-toasts
 * if the same version's "available" status re-renders (e.g. the parent
 * re-renders without a new check landing).
 */
export function UpdateToastTrigger({ automatic = true }: { automatic?: boolean }) {
  const { status, installAndRestart } = useUpdater({ automatic })
  const toast = useToast()
  const toastedVersionRef = useRef<string | null>(null)

  useEffect(() => {
    if (status.kind !== 'available') {
      return
    }
    const version = status.update.version
    if (toastedVersionRef.current === version) {
      return
    }
    toastedVersionRef.current = version
    // The menu bar panel shows it too, and sends the user here to install it.
    emitTo('tray', 'update-available', { version }).catch(() => {
      // No panel to tell; the toast is enough.
    })
    toast.show({
      tone: 'info',
      title: 'Update available',
      description: `Version ${version} is ready to install.`,
      durationMs: null,
      action: {
        label: 'Restart and install',
        onClick: () => {
          void installAndRestart()
        },
      },
    })
  }, [status, installAndRestart, toast])

  return null
}
