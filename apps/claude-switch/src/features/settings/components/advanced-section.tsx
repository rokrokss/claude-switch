import type { SegmentedOption } from '@/design'
import type { Settings } from '@/lib/types'

import { useState } from 'react'

import { Button, Dialog, Kbd, Segmented, Switch, useToast } from '@/design'
import { useAppState } from '@/lib/app-state/use-app-state'
import { getLogFile, openInFinder } from '@/lib/commands'
import { extractErrorMessage } from '@/lib/extract-error-message'
import { defaultSettings } from '@/lib/settings'

import { SettingRow, SettingsSection } from './settings-card'

type LogLevel = Settings['advanced']['logLevel']

const levelOptions: ReadonlyArray<SegmentedOption<LogLevel>> = [
  { value: 'error', label: 'Errors' },
  { value: 'warning', label: 'Warnings' },
  { value: 'info', label: 'Info' },
  { value: 'debug', label: 'Debug' },
]

/**
 * Update checks, the log, and putting the menu bar's and the panel's
 * settings back as they came.
 */
export function AdvancedSection() {
  const appState = useAppState()
  const { updates, advanced } = appState.state.settings
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
          title="Check for updates automatically"
          description="Looks for a new claude-switch at launch and every six hours. Off, only Settings → System checks."
          control={
            <Switch
              aria-label="Check for updates automatically"
              checked={updates.checkAutomatically}
              onChange={(checkAutomatically) =>
                void appState.update({ settings: { updates: { ...updates, checkAutomatically } } })
              }
            />
          }
        />
        <SettingRow
          title="Log"
          description="What claude-switch writes to its log. Tokens and your home folder never go in it."
          control={
            <div className="flex items-center gap-2">
              <Segmented<LogLevel>
                ariaLabel="Log level"
                size="small"
                options={levelOptions}
                value={advanced.logLevel}
                onChange={(logLevel) => void appState.update({ settings: { advanced: { ...advanced, logLevel } } })}
              />
              <Button
                size="sm"
                variant="secondary"
                onClick={() => {
                  getLogFile()
                    .then((path) => (path ? openInFinder(path) : undefined))
                    .catch((caught: unknown) => {
                      toast.error("Couldn't show the log.", extractErrorMessage(caught))
                    })
                }}
              >
                Reveal
              </Button>
            </div>
          }
        />
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
