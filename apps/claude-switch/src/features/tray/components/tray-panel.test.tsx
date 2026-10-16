import type { TrayEntry } from '@/lib/types'

import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import { openMainWindow, quitApp } from '@/lib/commands'

import { useTrayEntries } from '../api/use-tray-entries'
import { TrayPanel } from './tray-panel'

vi.mock('../api/use-tray-entries', () => ({ useTrayEntries: vi.fn() }))

vi.mock('@/lib/commands', () => ({
  loadAppState: vi.fn(() => Promise.resolve({ themeMode: 'system' })),
  openMainWindow: vi.fn(() => Promise.resolve()),
  quitApp: vi.fn(() => Promise.resolve()),
}))

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onFocusChanged: () => Promise.resolve(() => {}),
    setSize: () => Promise.resolve(),
  }),
  LogicalSize: class {},
}))

function makeEntry(overrides: Partial<TrayEntry> = {}): TrayEntry {
  return {
    id: 'work',
    app: 'claude',
    name: 'Work',
    command: 'claude-work',
    plan: null,
    metrics: [],
    usage: {
      quota: {
        primary: { utilization: 18, resetsAt: null },
        secondary: { utilization: 24, resetsAt: null },
        scopedWeekly: [],
      },
      quotaError: null,
      fetchedAt: new Date().toISOString(),
    },
    error: null,
    ...overrides,
  }
}

afterEach(() => {
  vi.clearAllMocks()
})

describe('TrayPanel', () => {
  it('shows each profile with its meters', () => {
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry(), makeEntry({ id: 'personal', name: 'Personal' })])
    render(<TrayPanel />)

    expect(screen.getByText('Work')).toBeInTheDocument()
    expect(screen.getByText('Personal')).toBeInTheDocument()
    expect(screen.getAllByText('18%')).toHaveLength(2)
    expect(screen.getAllByText('updated just now')).toHaveLength(2)
  })

  it('keeps the last meters and says the refresh failed', () => {
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry({ error: 'rate_limited' })])
    render(<TrayPanel />)

    expect(screen.getByText('18%')).toBeInTheDocument()
    expect(screen.getByText("Couldn't refresh — rate limited.")).toBeInTheDocument()
  })

  it('explains a profile with no usage yet, naming its command', () => {
    vi.mocked(useTrayEntries).mockReturnValue([makeEntry({ usage: null, error: 'unauthorized' })])
    render(<TrayPanel />)

    expect(screen.getByText('Token refresh needed — run `claude-work` once, then retry.')).toBeInTheDocument()
  })

  it('says it is loading until the first fetch finishes', () => {
    vi.mocked(useTrayEntries).mockReturnValue(null)
    render(<TrayPanel />)

    expect(screen.getByText('Loading usage…')).toBeInTheDocument()
  })

  it('opens the main window and quits', () => {
    vi.mocked(useTrayEntries).mockReturnValue([])
    render(<TrayPanel />)

    fireEvent.click(screen.getByRole('button', { name: 'Open claude-switch' }))
    fireEvent.click(screen.getByRole('button', { name: 'Quit' }))
    expect(openMainWindow).toHaveBeenCalledOnce()
    expect(quitApp).toHaveBeenCalledOnce()
  })
})
