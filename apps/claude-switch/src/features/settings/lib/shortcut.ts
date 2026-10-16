/**
 * The shortcut a key press makes, in the form the backend registers
 * (`Command+Shift+U`), or `null` when the press can't be one: it needs a
 * modifier other than Shift, and a letter, digit, function key or Space.
 */
export function acceleratorFrom(event: Pick<KeyboardEvent, 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'code'>) {
  const key = keyName(event.code)
  if (key === null || !(event.metaKey || event.ctrlKey || event.altKey)) {
    return null
  }
  const modifiers = [
    event.metaKey ? 'Command' : null,
    event.ctrlKey ? 'Control' : null,
    event.altKey ? 'Option' : null,
    event.shiftKey ? 'Shift' : null,
  ].filter((modifier): modifier is string => modifier !== null)
  return [...modifiers, key].join('+')
}

function keyName(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) {
    return code.slice(3)
  }
  if (/^Digit[0-9]$/.test(code)) {
    return code.slice(5)
  }
  if (/^F([1-9]|1[0-9]|20)$/.test(code) || code === 'Space') {
    return code
  }
  return null
}

const symbols: Record<string, string> = {
  Command: '⌘',
  Control: '⌃',
  Option: '⌥',
  Shift: '⇧',
  Space: 'Space',
}

/**
 * A shortcut as macOS menus print it: `⌘⇧U`.
 */
export function formatAccelerator(accelerator: string): string {
  return accelerator
    .split('+')
    .map((part) => symbols[part] ?? part)
    .join('')
}
