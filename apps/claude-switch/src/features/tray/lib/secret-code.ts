const konami = [
  'arrowup',
  'arrowup',
  'arrowdown',
  'arrowdown',
  'arrowleft',
  'arrowright',
  'arrowleft',
  'arrowright',
  'b',
  'a',
]

/**
 * The last keys pressed, as many as the Konami code (↑↑↓↓←→←→BA) is long,
 * with `key` added.
 */
export function remember(keys: Array<string>, key: string): Array<string> {
  return [...keys, key.toLowerCase()].slice(-konami.length)
}

/**
 * Whether the keys remembered spell the code.
 */
export function complete(keys: Array<string>): boolean {
  return keys.length === konami.length && keys.every((key, index) => key === konami[index])
}
