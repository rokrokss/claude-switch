import { describe, expect, it } from 'vitest'

import { complete, remember } from './secret-code'

const code = [
  'ArrowUp',
  'ArrowUp',
  'ArrowDown',
  'ArrowDown',
  'ArrowLeft',
  'ArrowRight',
  'ArrowLeft',
  'ArrowRight',
  'b',
  'a',
]

function type(keys: Array<string>) {
  return keys.reduce(remember, [] as Array<string>)
}

describe('the secret code', () => {
  it('completes on ↑↑↓↓←→←→BA, in either case', () => {
    expect(complete(type(code))).toBe(true)
    expect(complete(type([...code.slice(0, 8), 'B', 'A']))).toBe(true)
  })

  it('counts only the last keys pressed', () => {
    expect(complete(type(['ArrowUp', ...code]))).toBe(true)
    expect(complete(type([...code.slice(0, 9), 'x']))).toBe(false)
  })
})
