import { describe, expect, it } from 'vitest'

import { defaultSettings } from '@/lib/settings'

import { starsFor, toggleStar } from './stars'

const menuBar = defaultSettings.menuBar

describe('stars', () => {
  it('stars session and weekly until the user chooses', () => {
    expect(starsFor(menuBar, 'p1')).toEqual(['session', 'weekly'])
  })

  it('unstars a starred metric and stars another in its place', () => {
    const unstarred = toggleStar(menuBar, 'p1', 'weekly')
    expect(unstarred).toEqual({ menuBar: { ...menuBar, stars: { p1: ['session'] } } })
    if (!('menuBar' in unstarred)) {
      throw new Error('expected settings')
    }
    expect(toggleStar(unstarred.menuBar, 'p1', 'weekly.fable')).toEqual({
      menuBar: { ...menuBar, stars: { p1: ['session', 'weekly.fable'] } },
    })
  })

  it('refuses a third star and the trend', () => {
    expect(toggleStar(menuBar, 'p1', 'extra')).toEqual({ refused: 'Up to 2 stars per profile' })
    const one = { ...menuBar, stars: { p1: ['session'] } }
    expect(toggleStar(one, 'p1', 'trend')).toEqual({ refused: 'This one has no single value for the menu bar' })
  })
})
