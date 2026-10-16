import { describe, expect, it } from 'vitest'

import { acceleratorFrom, formatAccelerator } from './shortcut'

const press = (code: string, modifiers: Partial<Record<'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey', boolean>>) => ({
  code,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...modifiers,
})

describe('acceleratorFrom', () => {
  it('names the modifiers then the key', () => {
    expect(acceleratorFrom(press('KeyU', { metaKey: true, shiftKey: true }))).toBe('Command+Shift+U')
    expect(acceleratorFrom(press('Digit1', { ctrlKey: true, altKey: true }))).toBe('Control+Option+1')
    expect(acceleratorFrom(press('F5', { metaKey: true }))).toBe('Command+F5')
  })

  it('needs a modifier beyond Shift, and a key that can be one', () => {
    expect(acceleratorFrom(press('KeyU', {}))).toBeNull()
    expect(acceleratorFrom(press('KeyU', { shiftKey: true }))).toBeNull()
    expect(acceleratorFrom(press('MetaLeft', { metaKey: true }))).toBeNull()
  })
})

describe('formatAccelerator', () => {
  it('prints it the way menus do', () => {
    expect(formatAccelerator('Command+Shift+U')).toBe('⌘⇧U')
    expect(formatAccelerator('Control+Option+Space')).toBe('⌃⌥Space')
  })
})
