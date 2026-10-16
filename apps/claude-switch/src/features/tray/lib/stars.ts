import type { Settings } from '@/lib/types'

type MenuBar = Settings['menuBar']

/**
 * Two values stack beside a profile's mark in the menu bar; a third wouldn't fit.
 */
const maxStars = 2

/**
 * What a profile shows in the menu bar until the user stars something else.
 */
const defaultStars = ['session', 'weekly']

/**
 * Metrics that have no single value to show in the menu bar.
 */
const unstarrable = new Set(['trend'])

export function starsFor(menuBar: MenuBar, entryId: string): Array<string> {
  return menuBar.stars[entryId] ?? defaultStars
}

export function canStar(metricId: string): boolean {
  return !unstarrable.has(metricId)
}

/**
 * The menu bar settings with `metricId` starred or unstarred for the entry,
 * or why it can't be.
 */
export function toggleStar(
  menuBar: MenuBar,
  entryId: string,
  metricId: string,
): { menuBar: MenuBar } | { refused: string } {
  const stars = starsFor(menuBar, entryId)
  if (stars.includes(metricId)) {
    return {
      menuBar: withStars(
        menuBar,
        entryId,
        stars.filter((star) => star !== metricId),
      ),
    }
  }
  if (!canStar(metricId)) {
    return { refused: 'This one has no single value for the menu bar' }
  }
  if (stars.length >= maxStars) {
    return { refused: `Up to ${maxStars} stars per profile` }
  }
  return { menuBar: withStars(menuBar, entryId, [...stars, metricId]) }
}

function withStars(menuBar: MenuBar, entryId: string, stars: Array<string>): MenuBar {
  return { ...menuBar, stars: { ...menuBar.stars, [entryId]: stars } }
}
