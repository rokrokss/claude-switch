import type { Metric } from '@/lib/types'

import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'

import { SpendRow, TrendRow } from './spend-rows'

const today: Extract<Metric, { kind: 'values' }> = {
  id: 'today',
  label: 'Today',
  kind: 'values',
  values: [
    { amount: 12.34, unit: { type: 'money', currency: 'USD' } },
    { amount: 1_234_567, unit: { type: 'tokens' } },
  ],
  breakdown: {
    models: [
      { model: 'claude-fable-5-1', tokens: 1_000_000, cost: 10.5 },
      {
        model: 'Other',
        tokens: 234_567,
        cost: 1.84,
        variants: [
          { model: 'claude-haiku-5-5', tokens: 200_000, cost: 1.5 },
          { model: 'Unattributed', tokens: 34_567, cost: 0.34 },
        ],
      },
    ],
    unknownModels: ['mystery-1'],
    fallbackModels: ['gpt-5.5'],
  },
}

function trend(values: Array<number>): Extract<Metric, { kind: 'chart' }> {
  return {
    id: 'trend',
    label: 'Usage trend',
    kind: 'chart',
    unit: { type: 'tokens' },
    points: values.map((value, index) => ({ date: `2026-10-${String(index + 15).padStart(2, '0')}`, value })),
  }
}

describe('SpendRow', () => {
  it('reads cost and tokens, and opens to each model', () => {
    render(<SpendRow metric={today} spacing="" />)
    expect(screen.getByRole('button', { name: /Today/ })).toHaveTextContent(/12\.34 · 1\.2M tokens/)
    expect(screen.getByLabelText('A model has no price')).toBeInTheDocument()
    expect(screen.queryByText('claude-fable-5-1')).not.toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /Today/ }))
    expect(screen.getByText('claude-fable-5-1')).toBeInTheDocument()
    expect(screen.getByText('claude-haiku-5-5, Unattributed')).toBeInTheDocument()
    expect(screen.getByText(/No price for mystery-1/)).toBeInTheDocument()
    expect(screen.getByText(/Partly priced at gpt-5\.5 rates/)).toBeInTheDocument()
  })

  it('has no warning when every model is priced', () => {
    render(<SpendRow metric={{ ...today, breakdown: { models: [] } }} spacing="" />)
    expect(screen.queryByLabelText('A model has no price')).not.toBeInTheDocument()
  })
})

describe('TrendRow', () => {
  it('reads today, and the day pointed at', () => {
    const { container } = render(<TrendRow metric={trend([0, 3_400_000, 1_200_000])} spacing="" compact={false} />)
    expect(screen.getByText(/Today · 1\.2M tokens/)).toBeInTheDocument()
    expect(screen.getByRole('img')).toHaveAccessibleName(/the most on Oct 16: 3\.4M tokens/)

    const day = container.querySelector('[data-day="2026-10-16"]')
    if (!day) {
      throw new Error('no bar for Oct 16')
    }
    fireEvent.mouseEnter(day)
    expect(screen.getByText(/Oct 16 · 3\.4M tokens/)).toBeInTheDocument()
    fireEvent.mouseLeave(screen.getByRole('img'))
    expect(screen.getByText(/Today · 1\.2M tokens/)).toBeInTheDocument()
  })

  it('draws no bar for an idle day', () => {
    const { container } = render(<TrendRow metric={trend([0, 5])} spacing="" compact={false} />)
    const idle = container.querySelectorAll('[data-day="2026-10-15"] rect')[1]
    expect(idle).toHaveAttribute('height', '0')
  })
})
