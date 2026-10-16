import desktopPackage from '../../../claude-switch/package.json'

// The version of the desktop app, sourced at build time from
// apps/claude-switch/package.json. Prefix with `v` to match the
// GitHub Releases tag convention.
const desktopVersionRaw: string = desktopPackage.version
export const desktopVersion: string = `v${desktopVersionRaw}`

// Computed default .dmg filename. The actual filename comes from
// the GitHub release at runtime; this is the fallback shown
// before the JS swap completes.
export const defaultDmgFilename: string = `claude-switch_${desktopVersionRaw}_universal.dmg`
