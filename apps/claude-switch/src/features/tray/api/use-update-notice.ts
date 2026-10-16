import { useEffect, useState } from 'react'

import { listen } from '@tauri-apps/api/event'

/**
 * The version of an update the main window found, once it has found one.
 */
export function useUpdateNotice(): string | null {
  const [version, setVersion] = useState<string | null>(null)

  useEffect(() => {
    const unlistenPromise = listen<{ version: string }>('update-available', (event) => {
      setVersion(event.payload.version)
    })
    return () => {
      void unlistenPromise.then((unlisten) => {
        unlisten()
      })
    }
  }, [])

  return version
}
