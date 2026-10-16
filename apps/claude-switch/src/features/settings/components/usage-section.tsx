import type { SegmentedOption } from '@/design'
import type { Settings } from '@/lib/types'

import { Segmented, Switch } from '@/design'
import { useAppState } from '@/lib/app-state/use-app-state'

import { SettingRow, SettingsSection } from './settings-card'

type Display = Settings['display']

const meterOptions: ReadonlyArray<SegmentedOption<Display['meterStyle']>> = [
  { value: 'used', label: 'Used' },
  { value: 'left', label: 'Left' },
]

const resetOptions: ReadonlyArray<SegmentedOption<Display['resetDisplay']>> = [
  { value: 'countdown', label: 'Countdown' },
  { value: 'exact', label: 'Time' },
]

/**
 * How the menu bar and its panel read usage.
 */
export function UsageSection() {
  const appState = useAppState()
  const display = appState.state.settings.display

  function save(next: Display) {
    void appState.update({ settings: { display: next } })
  }

  return (
    <SettingsSection title="Usage">
      <SettingRow
        title="Show usage as"
        description="What a meter's figure says: how much is used, or how much is left."
        control={
          <Segmented<Display['meterStyle']>
            ariaLabel="Show usage as"
            size="small"
            options={meterOptions}
            value={display.meterStyle}
            onChange={(meterStyle) => save({ ...display, meterStyle })}
          />
        }
      />
      <SettingRow
        title="Resets"
        description="When a window resets: a countdown, or the time it happens."
        control={
          <Segmented<Display['resetDisplay']>
            ariaLabel="Resets"
            size="small"
            options={resetOptions}
            value={display.resetDisplay}
            onChange={(resetDisplay) => save({ ...display, resetDisplay })}
          />
        }
      />
      <SettingRow
        title="Always show pacing"
        description="Marks the even pace and where usage lands at the reset on every meter, not only those running close."
        control={
          <Switch
            aria-label="Always show pacing"
            checked={display.alwaysShowPacing}
            onChange={(alwaysShowPacing) => save({ ...display, alwaysShowPacing })}
          />
        }
      />
    </SettingsSection>
  )
}
