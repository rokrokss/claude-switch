import type { Settings, ThemeMode } from '@/lib/types'

import { useEffect, useState } from 'react'

import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { loadAppState, updateAppState } from '@/lib/commands'
import { defaultSettings } from '@/lib/settings'

type PanelSettings = {
  settings: Settings
  themeMode: ThemeMode
  /**
   * Saves the groups given; the backend tells every window.
   */
  update: (patch: Partial<Settings>) => void
}

/**
 * The settings and theme the panel draws with. Read again whenever the panel
 * gains focus (each time it opens), and replaced whenever a window changes
 * them, so a change made in the main window reaches it.
 */
export function usePanelSettings(): PanelSettings {
  const [settings, setSettings] = useState<Settings>(defaultSettings)
  const [themeMode, setThemeMode] = useState<ThemeMode>('system')

  useEffect(() => {
    function load() {
      void loadAppState().then((state) => {
        setSettings(state.settings)
        setThemeMode(state.themeMode)
      })
    }
    load()
    const unlistenFocus = getCurrentWindow().onFocusChanged((event) => {
      if (event.payload) {
        load()
      }
    })
    const unlistenSettings = listen<Settings>('settings-changed', (event) => {
      setSettings(event.payload)
    })
    return () => {
      for (const unlisten of [unlistenFocus, unlistenSettings]) {
        void unlisten.then((stop) => {
          stop()
        })
      }
    }
  }, [])

  function update(patch: Partial<Settings>) {
    // Shown at once; the backend's `settings-changed` confirms it.
    setSettings((current) => ({ ...current, ...patch }))
    void updateAppState({ settings: patch })
  }

  return { settings, themeMode, update }
}
