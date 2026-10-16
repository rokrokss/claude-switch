import type { SegmentedOption } from '@/design'
import type { Settings } from '@/lib/types'

import { Segmented, Switch } from '@/design'
import { useAppState } from '@/lib/app-state/use-app-state'

import { useLoginItem } from '../api/use-login-item'
import { SettingRow, SettingsSection } from './settings-card'
import { ShortcutRecorder } from './shortcut-recorder'

type Style = Settings['menuBar']['style']

const styleOptions: ReadonlyArray<SegmentedOption<Style>> = [
  { value: 'text', label: 'Values' },
  { value: 'bars', label: 'Bars' },
]

/**
 * Settings for the menu bar item: opening at login, how it looks, the
 * shortcut to its panel, and hiding it from an audience.
 */
export function MenuBarSection() {
  const appState = useAppState()
  const { menuBar, privacy } = appState.state.settings
  const login = useLoginItem()

  function save(patch: Partial<Settings>) {
    void appState.update({ settings: patch })
  }

  let loginNote = login.error
  if (login.status === 'needsApproval') {
    loginNote = 'Allow claude-switch in System Settings → General → Login Items.'
  } else if (login.status === 'unavailable') {
    loginNote = 'Only claude-switch installed in Applications can open at login.'
  }

  return (
    <SettingsSection title="Menu bar">
      <SettingRow
        title="Open at login"
        description="Starts claude-switch when you log in, so its menu bar item is always there."
        note={loginNote}
        control={
          <Switch
            aria-label="Open at login"
            checked={login.status === 'on' || login.status === 'needsApproval'}
            disabled={login.status === null || login.status === 'unavailable'}
            onChange={login.set}
          />
        }
      />
      <SettingRow
        title="Style"
        description="Each profile's mark beside its starred values, or one glyph of bars."
        control={
          <Segmented<Style>
            ariaLabel="Menu bar style"
            size="small"
            options={styleOptions}
            value={menuBar.style}
            onChange={(style) => save({ menuBar: { ...menuBar, style } })}
          />
        }
      />
      <SettingRow
        title="Shortcut"
        description="Opens the menu bar panel from anywhere."
        control={
          <ShortcutRecorder
            value={menuBar.shortcut}
            onChange={(shortcut) => save({ menuBar: { ...menuBar, shortcut } })}
          />
        }
      />
      <SettingRow
        title="Hide while screen sharing"
        description="Shows claude-switch's mark instead of usage while the screen is shared or recorded."
        control={
          <Switch
            aria-label="Hide while screen sharing"
            checked={privacy.hideWhileScreenSharing}
            onChange={(hideWhileScreenSharing) => save({ privacy: { ...privacy, hideWhileScreenSharing } })}
          />
        }
      />
    </SettingsSection>
  )
}
