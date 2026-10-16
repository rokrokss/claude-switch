import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { Switch } from './switch'

describe('Switch', () => {
  it('says whether it is on', () => {
    render(
      <>
        <Switch aria-label="On" checked onChange={() => {}} />
        <Switch aria-label="Off" checked={false} onChange={() => {}} />
      </>,
    )
    expect(screen.getByRole('switch', { name: 'On' })).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByRole('switch', { name: 'Off' })).toHaveAttribute('aria-checked', 'false')
  })

  it('asks for the opposite of its state when clicked', async () => {
    const handleChange = vi.fn()
    render(<Switch aria-label="Alerts" checked={false} onChange={handleChange} />)
    await userEvent.click(screen.getByRole('switch', { name: 'Alerts' }))
    expect(handleChange).toHaveBeenCalledWith(true)
  })

  it('can be named and described by visible text', () => {
    render(
      <>
        <span id="title">Almost out</span>
        <span id="detail">When little is left.</span>
        <Switch aria-labelledby="title" aria-describedby="detail" checked onChange={() => {}} />
      </>,
    )
    expect(screen.getByRole('switch', { name: 'Almost out', description: 'When little is left.' })).toBeInTheDocument()
  })

  it('does nothing while disabled', async () => {
    const handleChange = vi.fn()
    render(<Switch aria-label="Alerts" checked disabled onChange={handleChange} />)
    await userEvent.click(screen.getByRole('switch', { name: 'Alerts' }))
    expect(handleChange).not.toHaveBeenCalled()
  })
})
