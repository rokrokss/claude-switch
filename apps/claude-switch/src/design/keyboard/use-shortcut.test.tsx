import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { useShortcut } from './use-shortcut'

function Harness({ onCopy, onDelete, onPalette }: { onCopy: () => void; onDelete: () => void; onPalette: () => void }) {
  useShortcut('copy-selected-cli', onCopy, { enabled: true })
  useShortcut('delete-selected', onDelete, { enabled: true })
  useShortcut('toggle-palette', onPalette, { enabled: true })
  return (
    <>
      <input aria-label="Search" />
      <p>Selectable message</p>
    </>
  )
}

function setup() {
  const onCopy = vi.fn()
  const onDelete = vi.fn()
  const onPalette = vi.fn()
  render(<Harness onCopy={onCopy} onDelete={onDelete} onPalette={onPalette} />)
  return { onCopy, onDelete, onPalette, user: userEvent.setup() }
}

describe('useShortcut', () => {
  it('lets a text field keep the keys of a shortcut that acts on the selected profile', async () => {
    const { onCopy, onDelete, user } = setup()
    await user.click(screen.getByLabelText('Search'))
    await user.keyboard('{Meta>}c{/Meta}')
    await user.keyboard('{Meta>}{Backspace}{/Meta}')
    expect(onCopy).not.toHaveBeenCalled()
    expect(onDelete).not.toHaveBeenCalled()
  })

  it('still fires a shortcut on the selected profile outside a text field', async () => {
    const { onDelete, user } = setup()
    await user.keyboard('{Meta>}{Backspace}{/Meta}')
    await waitFor(() => expect(onDelete).toHaveBeenCalledTimes(1))
  })

  it('still fires an app-wide shortcut from a text field', async () => {
    const { onPalette, user } = setup()
    await user.click(screen.getByLabelText('Search'))
    await user.keyboard('{Meta>}k{/Meta}')
    await waitFor(() => expect(onPalette).toHaveBeenCalledTimes(1))
  })

  it('leaves ⌘C to the browser while text is selected', async () => {
    const { onCopy, user } = setup()
    const selection = window.getSelection()
    selection?.selectAllChildren(screen.getByText('Selectable message'))
    await user.keyboard('{Meta>}c{/Meta}')
    expect(onCopy).not.toHaveBeenCalled()
    selection?.removeAllRanges()
    await user.keyboard('{Meta>}c{/Meta}')
    await waitFor(() => expect(onCopy).toHaveBeenCalledTimes(1))
  })
})
