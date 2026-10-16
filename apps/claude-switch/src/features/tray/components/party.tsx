import { createContext, useContext, useEffect, useState } from 'react'

import { complete, remember } from '../lib/secret-code'

const partyKey = 'claude-switch-party'

/**
 * Whether the panel is in party mode: meters fill with a moving rainbow.
 */
export const PartyContext = createContext(false)

export function useParty(): boolean {
  return useContext(PartyContext)
}

/**
 * Party mode, toggled by typing the Konami code (↑↑↓↓←→←→BA) in the panel
 * and remembered between launches.
 */
export function usePartyMode(): boolean {
  const [party, setParty] = useState(() => {
    try {
      return window.localStorage.getItem(partyKey) === 'on'
    } catch {
      return false
    }
  })

  useEffect(() => {
    let keys: Array<string> = []
    function onKey(event: KeyboardEvent) {
      if (event.repeat || event.metaKey || event.ctrlKey || event.altKey) {
        return
      }
      keys = remember(keys, event.key)
      if (complete(keys)) {
        keys = []
        setParty((current) => {
          try {
            window.localStorage.setItem(partyKey, current ? 'off' : 'on')
          } catch {
            // It lasts until the panel reloads; nothing else depends on it.
          }
          return !current
        })
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  return party
}
