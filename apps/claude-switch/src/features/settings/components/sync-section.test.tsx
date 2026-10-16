import type { AppState, AppStatePatch, ICloudSync } from '@/lib/types'

import { invoke } from '@tauri-apps/api/core'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { defaultSettings } from '@/lib/settings'
import { renderWithQuery } from '@/test/render-with-query'

import { SyncSection } from './sync-section'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => undefined) }))

const mockInvoke = vi.mocked(invoke)

const OFF: ICloudSync = { on: false, syncing: false, problem: null, unreadable: [], devices: [] }

/**
 * Answers the section's commands from a little state of its own, so turning sync on shows on the
 * next render.
 */
function prime(icloud: boolean, status: ICloudSync) {
  let appState = {
    welcomeShown: true,
    settings: { ...defaultSettings, sync: { icloud } },
  } as unknown as AppState
  mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
    if (command === 'icloud_sync') {
      return status
    }
    if (command === 'load_app_state') {
      return appState
    }
    if (command === 'update_app_state') {
      const { patch } = args as { patch: AppStatePatch }
      appState = { ...appState, settings: { ...appState.settings, ...patch.settings } }
      return appState
    }
    throw new Error(`unexpected command in test: ${command}`)
  })
}

beforeEach(() => {
  mockInvoke.mockReset()
})

describe('SyncSection', () => {
  it('turns sync on from its switch', async () => {
    prime(false, OFF)
    renderWithQuery(<SyncSection />)
    const user = userEvent.setup()
    const toggle = await screen.findByRole('switch', { name: 'Sync usage across Macs' })
    expect(toggle).not.toBeChecked()
    await user.click(toggle)
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('update_app_state', {
        patch: { settings: { sync: { icloud: true } } },
      }),
    )
  })

  it('lists the Macs, and what each adds to which profile', async () => {
    const now = new Date().toISOString()
    prime(true, {
      ...OFF,
      on: true,
      devices: [
        { id: 'a', name: 'Studio', updatedAt: now, thisMac: true, profiles: ['Work'] },
        { id: 'b', name: 'Air', updatedAt: now, thisMac: false, profiles: ['Work', 'Personal'] },
        { id: 'c', name: 'Old iMac', updatedAt: now, thisMac: false, profiles: [] },
      ],
    })
    renderWithQuery(<SyncSection />)
    expect(await screen.findByText('Studio')).toBeInTheDocument()
    expect(screen.getByText('This Mac')).toBeInTheDocument()
    expect(screen.getByText('Sharing Work')).toBeInTheDocument()
    expect(screen.getByText('Adds to Work, Personal')).toBeInTheDocument()
    expect(screen.getByText('No account in common with this Mac')).toBeInTheDocument()
  })

  it('says what keeps sync from working, and which files it left out', async () => {
    prime(true, {
      ...OFF,
      on: true,
      problem: "Some Macs' usage couldn't be read; they are left out.",
      unreadable: ['x.json: expected value at line 1 column 1'],
    })
    renderWithQuery(<SyncSection />)
    const notice = await screen.findByRole('status')
    expect(notice).toHaveTextContent("Some Macs' usage couldn't be read")
    expect(notice).toHaveTextContent('x.json: expected value')
  })
})
