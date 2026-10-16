import { useState } from 'react'

import { Button, Dialog, Kbd, useToast } from '@/design'
import { useAppState } from '@/lib/app-state/use-app-state'
import { defaultSettings } from '@/lib/settings'

import { SettingRow, SettingsSection } from './settings-card'

/**
 * Putting the menu bar's and the panel's settings back as they came.
 */
export function AdvancedSection() {
  const appState = useAppState()
  const toast = useToast()
  const [confirmOpen, setConfirmOpen] = useState(false)
  const [submitting, setSubmitting] = useState(false)

  async function reset() {
    if (submitting) {
      return
    }
    setSubmitting(true)
    try {
      await appState.update({ settings: defaultSettings })
      toast.success('Settings reset.', 'The menu bar and its panel are back as they came.')
      setConfirmOpen(false)
    } catch (caught) {
      toast.error("Couldn't reset the settings.", caught instanceof Error ? caught.message : String(caught))
    } finally {
      setSubmitting(false)
    }
  }

  return (
    <>
      <SettingsSection title="Advanced">
        <SettingRow
          title="Reset all settings"
          description="Stars, card layouts, the shortcut and every option above go back to how they came. Profiles and sign-ins stay."
          control={
            <Button size="sm" variant="danger" onClick={() => setConfirmOpen(true)}>
              Reset…
            </Button>
          }
        />
      </SettingsSection>
      <Dialog
        open={confirmOpen}
        title="Reset all settings?"
        description="The menu bar, its panel, notifications and the rest go back to their defaults. This can't be undone."
        onClose={() => setConfirmOpen(false)}
        onSubmit={reset}
        foot={
          <>
            <Button
              disabled={submitting}
              variant="ghost"
              size="sm"
              trailingKbd={<Kbd>⎋</Kbd>}
              onClick={() => setConfirmOpen(false)}
            >
              Cancel
            </Button>
            <Button
              disabled={submitting}
              variant="primary"
              size="sm"
              trailingKbd={<Kbd variant="onAccent">⏎</Kbd>}
              onClick={reset}
            >
              Reset
            </Button>
          </>
        }
      >
        <p className="text-body text-ink-soft">Profiles, their sign-ins and their data are untouched.</p>
      </Dialog>
    </>
  )
}
