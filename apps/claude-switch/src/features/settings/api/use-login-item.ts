import type { LoginItem } from '@/lib/types'

import { useEffect, useState } from 'react'

import { getLoginItem, setLoginItem } from '@/lib/commands'
import { extractErrorMessage } from '@/lib/extract-error-message'

type LoginItemState = {
  /**
   * As macOS has it; `null` until read.
   */
  status: LoginItem | null
  error: string | null
  set: (enabled: boolean) => void
}

/**
 * Whether claude-switch opens at login, read from macOS (the user can change
 * it in System Settings too), and the switch that changes it.
 */
export function useLoginItem(): LoginItemState {
  const [status, setStatus] = useState<LoginItem | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    void getLoginItem().then(setStatus)
  }, [])

  function set(enabled: boolean) {
    setError(null)
    setLoginItem(enabled)
      .then(setStatus)
      .catch((caught: unknown) => {
        setError(extractErrorMessage(caught))
      })
  }

  return { status, error, set }
}
