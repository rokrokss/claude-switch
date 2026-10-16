import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { Switch } from './switch'

describe('Switch', () => {
  it('reports its state and flips on a click', () => {
    const onChange = vi.fn()
    render(<Switch checked={false} ariaLabel="Open at login" onChange={onChange} />)
    const control = screen.getByRole('switch', { name: 'Open at login' })
    expect(control).toHaveAttribute('aria-checked', 'false')
    fireEvent.click(control)
    expect(onChange).toHaveBeenCalledWith(true)
  })

  it('does nothing while disabled', () => {
    const onChange = vi.fn()
    render(<Switch checked ariaLabel="Open at login" disabled onChange={onChange} />)
    fireEvent.click(screen.getByRole('switch'))
    expect(onChange).not.toHaveBeenCalled()
  })
})
