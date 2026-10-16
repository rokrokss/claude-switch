import type { SegmentedOption, ThemeMode } from '@/design'
import type { Settings } from '@/lib/types'

import { Monitor, Moon, Sun } from 'lucide-react'

import { Segmented, Switch, useTheme } from '@/design'
import { useAppState } from '@/lib/app-state/use-app-state'

import { SettingRow } from './settings-card'

type Display = Settings['display']

const densityOptions: ReadonlyArray<SegmentedOption<Display['density']>> = [
  { value: 'default', label: 'Default' },
  { value: 'compact', label: 'Compact' },
]

const timeOptions: ReadonlyArray<SegmentedOption<Display['timeFormat']>> = [
  { value: 'auto', label: 'System' },
  { value: '12h', label: '12-hour' },
  { value: '24h', label: '24-hour' },
]

const themeOptions: ReadonlyArray<SegmentedOption<ThemeMode>> = [
  {
    value: 'light',
    label: 'Light',
    ariaLabel: 'Light theme',
    icon: <Sun className="h-[13px] w-[13px]" strokeWidth={1.75} />,
  },
  {
    value: 'system',
    label: 'System',
    ariaLabel: 'System theme',
    icon: <Monitor className="h-[13px] w-[13px]" strokeWidth={1.75} />,
  },
  {
    value: 'dark',
    label: 'Dark',
    ariaLabel: 'Dark theme',
    icon: <Moon className="h-[13px] w-[13px]" strokeWidth={1.75} />,
  },
]

/**
 * Top section of Settings — theme picker.
 *
 * The segmented control's value mirrors `useTheme().mode` (the live in-memory
 * mode, which the ThemeProvider applies to `<html data-theme>`). Changes flow
 * through two writes:
 * 1. `theme.setMode(next)` — flips the provider immediately so the whole app
 *    repaints with no waiting on the IPC round-trip.
 * 2. `appState.update({ themeMode: next })` — persists the choice to
 *    state.json via the existing optimistic mutation. The corresponding
 *    effect in `app.tsx` becomes a no-op since the two values are already
 *    in sync.
 */
export function AppearanceSection() {
  const theme = useTheme()
  const appState = useAppState()

  function handleChange(next: ThemeMode) {
    theme.setMode(next)
    void appState.update({ themeMode: next })
  }

  return (
    <section className="mb-8">
      <div className="mb-2.5 flex items-center justify-between">
        <span className="font-mono text-[10px] font-medium uppercase tracking-[0.1em] text-muted-strong">
          Appearance
        </span>
      </div>
      <Segmented<ThemeMode> ariaLabel="Theme" options={themeOptions} value={theme.mode} onChange={handleChange} />
      <MenuBarPanelAppearance />
    </section>
  )
}

/**
 * How the menu bar's panel is laid out and moves.
 */
function MenuBarPanelAppearance() {
  const appState = useAppState()
  const display = appState.state.settings.display

  function save(next: Display) {
    void appState.update({ settings: { display: next } })
  }

  return (
    <div className="mt-3 rounded-xl border border-border bg-white dark:bg-cream-2">
      <SettingRow
        title="Panel density"
        description="Compact fits more profiles in the menu bar's panel."
        control={
          <Segmented<Display['density']>
            ariaLabel="Panel density"
            size="small"
            options={densityOptions}
            value={display.density}
            onChange={(density) => save({ ...display, density })}
          />
        }
      />
      <SettingRow
        title="Time format"
        description="How the panel prints times of day."
        control={
          <Segmented<Display['timeFormat']>
            ariaLabel="Time format"
            size="small"
            options={timeOptions}
            value={display.timeFormat}
            onChange={(timeFormat) => save({ ...display, timeFormat })}
          />
        }
      />
      <SettingRow
        title="Reduce animations"
        description="Stills the panel's spinners and transitions. macOS's own Reduce Motion does the same."
        control={
          <Switch
            aria-label="Reduce animations"
            checked={display.reduceAnimations}
            onChange={(reduceAnimations) => save({ ...display, reduceAnimations })}
          />
        }
      />
    </div>
  )
}
