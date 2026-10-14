import changelog from '../../../../CHANGELOG.md?raw'
import { parseChangelog } from './changelog'

/**
 * Release notes parsed from `apps/claude-switch/CHANGELOG.md`, inlined at build time, so a build
 * always contains its own entry. Parsed once at module load.
 */
export const bundledReleases = parseChangelog(changelog)
