import type { Settings } from './types'

/**
 * The settings of a fresh install. Mirrors `Settings::default()` in
 * src-tauri/src/settings.rs.
 */
export const defaultSettings: Settings = {
  menuBar: { style: 'text', stars: {}, shortcut: null },
  display: {
    meterStyle: 'used',
    resetDisplay: 'countdown',
    alwaysShowPacing: false,
    timeFormat: 'auto',
    density: 'default',
    reduceAnimations: false,
    increaseTransparency: false,
    totalSpend: { shown: true, metric: 'cost', period: 'today' },
  },
  layout: { profiles: {}, expanded: [] },
  notifications: { almostOut: false, cuttingItClose: false, willRunOut: false },
  privacy: { hideWhileScreenSharing: false },
  integrations: { localApi: false },
  sync: { icloud: false },
  updates: { checkAutomatically: true, beta: false },
  advanced: { logLevel: 'info', codexFallbackModel: null },
}
