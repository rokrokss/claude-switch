import { useState } from 'react'

type SignedCloneOption = {
  /**
   * Whether the option is chosen, before it is narrowed to what can be saved.
   */
  chosen: boolean
  /**
   * Sets the option outright, as when the form is reset.
   */
  set: (next: boolean) => void
  /**
   * Turns the option on or off. Turning it on turns the Dock icon option off.
   */
  choose: (next: boolean) => void
  /**
   * Wraps the Dock icon option's setter so that turning that on turns this off.
   */
  withDockIcon: (setDockIcon: (next: boolean) => void) => (next: boolean) => void
  /**
   * Whether the explanation is on screen.
   */
  explaining: boolean
  explain: () => void
  closeExplanation: () => void
}

/**
 * The signed clone option of a profile form, and its explanation.
 *
 * A signed clone and a Dock icon of its own are different launchers at the
 * same path, so the two options clear each other. `clearDockIcon` turns the
 * Dock icon option off.
 */
export function useSignedCloneOption(initial: boolean, clearDockIcon: () => void): SignedCloneOption {
  const [chosen, setChosen] = useState(initial)
  const [explaining, setExplaining] = useState(false)

  return {
    chosen,
    set: setChosen,
    choose: (next) => {
      setChosen(next)
      if (next) {
        clearDockIcon()
      }
    },
    withDockIcon: (setDockIcon) => (next) => {
      setDockIcon(next)
      if (next) {
        setChosen(false)
      }
    },
    explaining,
    explain: () => setExplaining(true),
    closeExplanation: () => setExplaining(false),
  }
}
