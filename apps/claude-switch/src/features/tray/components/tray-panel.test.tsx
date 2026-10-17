import type { Metric, Settings, TrayEntry } from '@/lib/types'

import { fireEvent, render, screen, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { claimCodexReset, closePanel, openSettings, quitApp, refreshUsage } from '@/lib/commands'
import { defaultSettings } from '@/lib/settings'

import { usePanelSettings } from '../api/use-panel-settings'
import { useTrayEntries } from '../api/use-tray-entries'
import { useTrayStatus } from '../api/use-tray-status'
import { useUpdateNotice } from '../api/use-update-notice'
import { keyAction, TrayPanel } from './tray-panel'

vi.mock('../api/use-tray-entries', () => ({ useTrayEntries: vi.fn() }))
vi.mock('../api/use-tray-status', () => ({ useTrayStatus: vi.fn() }))
vi.mock('../api/use-panel-settings', () => ({ usePanelSettings: vi.fn() }))
vi.mock('../api/use-update-notice', () => ({ useUpdateNotice: vi.fn() }))

vi.mock('@/lib/commands', () => ({
  claimCodexReset: vi.fn(() => Promise.resolve('reset')),
  closePanel: vi.fn(() => Promise.resolve()),
  getAppMetadata: vi.fn(() => Promise.resolve({ version: '2.0.0' })),
  openAbout: vi.fn(() => Promise.resolve()),
  openExternalUrl: vi.fn(() => Promise.resolve()),
  openMainWindow: vi.fn(() => Promise.resolve()),
  openSettings: vi.fn(() => Promise.resolve()),
  quitApp: vi.fn(() => Promise.resolve()),
  refreshUsage: vi.fn(() => Promise.resolve()),
}))

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ setSize: () => Promise.resolve() }),
  LogicalSize: class {},
}))

type Meter = Extract<Metric, { kind: 'meter' }>

const hour = 3_600_000

function meter(id: string, label: string, used: number, overrides: Partial<Meter> = {}): Meter {
  return {
    id,
    label,
    kind: 'meter',
    used,
    limit: 100,
    unit: { type: 'percent' },
    resetsAt: new Date(Date.now() + 3 * hour + 25 * 60_000 - 1_000).toISOString(),
    periodSecs: 5 * 3600,
    notStarted: false,
    state: { kind: 'healthy', projectedFraction: 0.4 },
    ...overrides,
  }
}

const resets: Metric = {
  id: 'resets',
  label: 'Rate limit resets',
  kind: 'values',
  values: [{ amount: 3, unit: { type: 'count' } }],
}

function makeEntry(overrides: Partial<TrayEntry> = {}): TrayEntry {
  return {
    id: 'work',
    app: 'claude',
    name: 'Work',
    command: 'claude-work',
    plan: 'Max 20x',
    usage: { quota: null, quotaError: null, fetchedAt: new Date().toISOString() },
    metrics: [meter('session', 'Session', 18), meter('weekly', 'Weekly', 24)],
    error: null,
    ...overrides,
  }
}

const update = vi.fn()

function withSettings(settings: Settings = defaultSettings) {
  vi.mocked(usePanelSettings).mockReturnValue({
    settings,
    themeMode: 'system',
    hiddenFromMenuBar: [],
    update,
  })
}

beforeEach(() => {
  vi.mocked(useTrayStatus).mockReturnValue({ refreshing: false, nextRefreshAt: null })
  vi.mocked(useUpdateNotice).mockReturnValue(null)
  withSettings()
  try {
    window.localStorage.setItem('claude-switch-tray-hint-dismissed', 'yes')
  } catch {
    // Without storage the hint shows, which no test here minds.
  }
})

afterEach(() => {
  vi.clearAllMocks()
})

/**
 * What a profile spent today.
 */
function today(cost: number, tokens: number): Metric {
  return {
    id: 'today',
    label: 'Today',
    kind: 'values',
    values: [
      { amount: cost, unit: { type: 'money', currency: 'USD' } },
      { amount: tokens, unit: { type: 'tokens' } },
    ],
  }
}

describe('TrayPanel', () => {
  it('shows each profile with its plan and meters', () => {
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry(), makeEntry({ id: 'personal', name: 'Personal' })])
    render(<TrayPanel />)
    expect(screen.getByText('Work')).toBeInTheDocument()
    expect(screen.getByText('Personal')).toBeInTheDocument()
    expect(screen.getAllByText('Max 20x')).toHaveLength(2)
    expect(screen.getAllByText('18% used')).toHaveLength(2)
    expect(screen.getAllByText('Resets in 3h 25m')).toHaveLength(4)
  })

  it('totals what every profile spent, and keeps the period chosen', () => {
    vi.mocked(useTrayEntries).mockReturnValue([
      makeEntry({ metrics: [today(6, 2_000_000)] }),
      makeEntry({ id: 'personal', name: 'Personal', metrics: [today(2, 6_000_000)] }),
    ])
    render(<TrayPanel />)
    const card = screen.getByRole('region', { name: 'Total spend' })
    expect(within(card).getByText(/8\.00$/)).toBeInTheDocument()
    expect(within(card).getByRole('img')).toHaveAccessibleName('Share of cost: Work 75%, Personal 25%')
    fireEvent.click(within(card).getByRole('radio', { name: 'Yesterday' }))
    expect(update).toHaveBeenCalledWith({
      display: {
        ...defaultSettings.display,
        totalSpend: { ...defaultSettings.display.totalSpend, period: 'yesterday' },
      },
    })
  })

  it('says when a rate-limited profile is asked again', () => {
    const retryAt = new Date(Date.now() + hour).toISOString()
    vi.mocked(useTrayEntries).mockReturnValue([
      makeEntry({ error: 'rate_limited', retryAt }),
      makeEntry({ id: 'fresh', name: 'Fresh', metrics: [], usage: null, error: 'rate_limited', retryAt }),
    ])
    render(<TrayPanel />)
    expect(screen.getAllByText(/^Rate limited · trying again (today|tomorrow) at /)).toHaveLength(2)
    // The note stands in for the card's bare "rate limited" message.
    expect(screen.queryByText(/Try again in a few minutes/)).not.toBeInTheDocument()
  })

  it('has no total spend card without spend', () => {
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry()])
    render(<TrayPanel />)
    expect(screen.queryByRole('region', { name: 'Total spend' })).not.toBeInTheDocument()
  })

  it('flips a meter between used and left', () => {
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry()])
    render(<TrayPanel />)
    fireEvent.click(screen.getByText('18% used'))
    expect(update).toHaveBeenCalledWith({ display: { ...defaultSettings.display, meterStyle: 'left' } })
  })

  it('reads a meter as left when set to', () => {
    withSettings({ ...defaultSettings, display: { ...defaultSettings.display, meterStyle: 'left' } })
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry()])
    render(<TrayPanel />)
    expect(screen.getByText('82% left')).toBeInTheDocument()
  })

  it('warns when a meter runs out, is close, or is spent', () => {
    vi.mocked(useTrayEntries).mockReturnValue([
      makeEntry({
        metrics: [
          meter('session', 'Session', 60, {
            state: {
              kind: 'runningOut',
              runsOutAt: new Date(Date.now() + 2 * hour - 1_000).toISOString(),
              projectedFraction: 1.2,
            },
          }),
          meter('weekly', 'Weekly', 47, { state: { kind: 'closeToLimit', sparePercent: 6, projectedFraction: 0.94 } }),
          meter('weekly.fable', 'Weekly · Fable', 100, { state: { kind: 'spent' } }),
        ],
      }),
    ])
    render(<TrayPanel />)
    expect(screen.getByText('Limit in 2h 0m')).toBeInTheDocument()
    expect(screen.getByText('~6% spare')).toBeInTheDocument()
    expect(screen.getByText('Limit reached')).toBeInTheDocument()
  })

  it('says a session has not started', () => {
    vi.mocked(useTrayEntries).mockReturnValue([
      makeEntry({ metrics: [meter('session', 'Session', 0, { resetsAt: null, notStarted: true })] }),
    ])
    render(<TrayPanel />)
    expect(screen.getByText('Not started')).toBeInTheDocument()
  })

  it('explains a profile with nothing to show, naming its command', () => {
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry({ usage: null, metrics: [], error: 'unauthorized' })])
    render(<TrayPanel />)
    expect(screen.getByText('Token refresh needed — run `claude-work` once, then retry.')).toBeInTheDocument()
  })

  it('shows a placed metric the profile does not report as having no data', () => {
    withSettings({
      ...defaultSettings,
      layout: { ...defaultSettings.layout, profiles: { work: { alwaysVisible: ['session', 'today'], onDemand: [] } } },
    })
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry()])
    render(<TrayPanel />)
    expect(screen.getByText('Today')).toBeInTheDocument()
    expect(screen.getByText('No data')).toBeInTheDocument()
    expect(screen.queryByText('24% used')).not.toBeInTheDocument()
  })

  it('opens a card to its on-demand metrics', () => {
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry({ metrics: [meter('session', 'Session', 18), resets] })])
    render(<TrayPanel />)
    expect(screen.queryByText('3 available')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Show more' }))
    expect(update).toHaveBeenCalledWith({ layout: { ...defaultSettings.layout, expanded: ['work'] } })
  })

  it('lists on-demand metrics and links once open', () => {
    withSettings({ ...defaultSettings, layout: { ...defaultSettings.layout, expanded: ['work'] } })
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry({ metrics: [meter('session', 'Session', 18), resets] })])
    render(<TrayPanel />)
    expect(screen.getByText('3 available')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Status' })).toBeInTheDocument()
  })

  it('uses a Codex reset after a confirmation', async () => {
    const lapses = new Date(Date.now() + 3 * 24 * hour).toISOString()
    withSettings({ ...defaultSettings, layout: { ...defaultSettings.layout, expanded: ['codex'] } })
    vi.mocked(useTrayEntries).mockReturnValue([
      makeEntry({
        id: 'codex',
        app: 'codex',
        name: 'ChatGPT',
        metrics: [
          {
            id: 'resets',
            label: 'Rate limit resets',
            kind: 'values',
            values: [{ amount: 1, unit: { type: 'count' } }],
            expiresAt: [lapses],
          },
        ],
      }),
    ])
    render(<TrayPanel />)
    fireEvent.click(screen.getByText('Rate limit resets'))
    fireEvent.click(screen.getByRole('button', { name: 'Use' }))
    expect(claimCodexReset).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Reset' }))
    expect(claimCodexReset).toHaveBeenCalledWith('codex', Math.floor(Date.parse(lapses) / 1000))
    expect(await screen.findByRole('status')).toHaveTextContent('Reset used: your Codex limits start over.')
  })

  it('throws a party on the Konami code', () => {
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry()])
    render(<TrayPanel />)
    const fill = () => screen.getByRole('progressbar', { name: 'Session' }).firstElementChild
    expect(fill()).not.toHaveClass('party-fill')
    for (const key of [
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
    ]) {
      fireEvent.keyDown(window, { key })
    }
    expect(fill()).toHaveClass('party-fill')
  })

  it('says when the next update comes, and updates on a click', async () => {
    vi.mocked(useTrayStatus).mockReturnValue({
      refreshing: false,
      nextRefreshAt: new Date(Date.now() + 9 * 60_000).toISOString(),
    })
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry()])
    render(<TrayPanel />)
    const footer = await screen.findByText('claude-switch 2.0.0 · Next update in 9m')
    fireEvent.click(footer)
    expect(refreshUsage).toHaveBeenCalledOnce()
  })

  it('offers an update the main window found', () => {
    vi.mocked(useUpdateNotice).mockReturnValue('2.1.0')
    vi.mocked(useTrayEntries).mockReturnValue([])
    render(<TrayPanel />)
    expect(screen.getByText('claude-switch 2.1.0 is ready to install.')).toBeInTheDocument()
  })

  it('says it is loading until the first fetch, and what to do with no profiles', () => {
    vi.mocked(useTrayEntries).mockReturnValue(null)
    const { rerender } = render(<TrayPanel />)
    expect(screen.getByText('Loading usage…')).toBeInTheDocument()
    vi.mocked(useTrayEntries).mockReturnValue([])
    rerender(<TrayPanel />)
    expect(screen.getByRole('button', { name: 'Open claude-switch' })).toBeInTheDocument()
  })

  it('closes on Escape, updates on ⌘R and opens Settings on ⌘,', () => {
    vi.mocked(useTrayEntries).mockReturnValue([])
    render(<TrayPanel />)
    fireEvent.keyDown(window, { key: 'Escape' })
    fireEvent.keyDown(window, { key: 'r', metaKey: true })
    fireEvent.keyDown(window, { key: ',', metaKey: true })
    expect(closePanel).toHaveBeenCalledOnce()
    expect(refreshUsage).toHaveBeenCalledOnce()
    expect(openSettings).toHaveBeenCalledOnce()
    expect(quitApp).not.toHaveBeenCalled()
  })
})

describe('keyAction', () => {
  function press(init: KeyboardEventInit, target?: Element) {
    const event = new KeyboardEvent('keydown', init)
    if (target) {
      Object.defineProperty(event, 'target', { value: target })
    }
    return keyAction(event)
  }

  it('maps the panel shortcuts', () => {
    expect(press({ key: 'Escape' })).toBe(closePanel)
    expect(press({ key: 'r', metaKey: true })).toBe(refreshUsage)
    expect(press({ key: ',', metaKey: true })).toBe(openSettings)
    expect(press({ key: 'Enter' }, document.body)).toBe(openSettings)
    expect(press({ key: 'r' })).toBeNull()
  })

  it('leaves keys typed into a field or a menu alone', () => {
    const input = document.createElement('input')
    document.body.append(input)
    expect(press({ key: 'Escape' }, input)).toBeNull()
    const button = document.createElement('button')
    document.body.append(button)
    expect(press({ key: 'Enter' }, button)).toBeNull()
  })
})
