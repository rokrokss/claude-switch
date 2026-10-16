import type { AppState, NotificationPermission } from '@/lib/types'

import { invoke } from '@tauri-apps/api/core'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { defaultSettings } from '@/lib/settings'
import { renderWithQuery } from '@/test/render-with-query'

import { NotificationsSection } from './notifications-section'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const mockInvoke = vi.mocked(invoke)

const OFF = { almostOut: false, cuttingItClose: false, willRunOut: false }

function stateWith(notifications: AppState['settings']['notifications']): AppState {
  return {
    welcomeShown: true,
    migrationDismissedAt: null,
    pathBannerDismissedAt: null,
    themeMode: 'system',
    selectedEntryId: null,
    dockIconAcknowledgedAt: null,
    defaultProfileNames: {},
    dismissedRepairSessions: {},
    hiddenFromMenuBar: [],
    settings: { ...defaultSettings, notifications },
  }
}

/**
 * Answers the section's commands: the app state, macOS's permission (`permission` until asked,
 * `afterRequest` once asked), and the writes.
 */
function prime(
  notifications: AppState['settings']['notifications'],
  permission: NotificationPermission = 'granted',
  afterRequest: NotificationPermission = permission,
) {
  let current = stateWith(notifications)
  let asked = false
  mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
    switch (command) {
      case 'load_app_state':
        return current
      case 'update_app_state': {
        const { patch } = args as { patch: { settings: Partial<AppState['settings']> } }
        current = { ...current, settings: { ...current.settings, ...patch.settings } }
        return current
      }
      case 'notification_permission':
        return asked ? afterRequest : permission
      case 'request_notification_permission':
        asked = true
        return afterRequest
      case 'open_notification_settings':
        return null
      default:
        throw new Error(`unexpected command in test: ${command}`)
    }
  })
}

function invoked(command: string) {
  return mockInvoke.mock.calls.filter(([name]) => name === command)
}

beforeEach(() => {
  mockInvoke.mockReset()
})

describe('NotificationsSection', () => {
  it('shows a switch per trigger, each as set', async () => {
    prime({ ...OFF, cuttingItClose: true })
    renderWithQuery(<NotificationsSection />)
    expect(
      await screen.findByRole('switch', { name: 'Almost out', description: 'When less than 10% of a limit is left.' }),
    ).toHaveAttribute('aria-checked', 'false')
    expect(screen.getByRole('switch', { name: 'Cutting it close' })).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByRole('switch', { name: 'Will run out' })).toHaveAttribute('aria-checked', 'false')
  })

  it('does not read the permission, nor warn, while every trigger is off', async () => {
    prime(OFF, 'denied')
    renderWithQuery(<NotificationsSection />)
    await screen.findByRole('switch', { name: 'Almost out' })
    expect(invoked('notification_permission')).toHaveLength(0)
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })

  it('saves the whole group and asks macOS when the first trigger goes on', async () => {
    prime(OFF, 'notDetermined', 'granted')
    renderWithQuery(<NotificationsSection />)
    await userEvent.click(await screen.findByRole('switch', { name: 'Will run out' }))
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('update_app_state', {
        patch: { settings: { notifications: { ...OFF, willRunOut: true } } },
      }),
    )
    await waitFor(() => expect(invoked('request_notification_permission')).toHaveLength(1))
    await waitFor(() => expect(screen.getByRole('switch', { name: 'Will run out' })).toBeChecked())
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })

  it('does not ask again when another trigger goes on', async () => {
    prime({ ...OFF, almostOut: true })
    renderWithQuery(<NotificationsSection />)
    await userEvent.click(await screen.findByRole('switch', { name: 'Cutting it close' }))
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('update_app_state', {
        patch: { settings: { notifications: { ...OFF, almostOut: true, cuttingItClose: true } } },
      }),
    )
    expect(invoked('request_notification_permission')).toHaveLength(0)
  })

  it('points to System Settings when macOS blocks the notifications', async () => {
    prime({ ...OFF, almostOut: true }, 'denied')
    renderWithQuery(<NotificationsSection />)
    expect(await screen.findByRole('status')).toHaveTextContent(
      'macOS is set to block notifications from claude-switch.',
    )
    await userEvent.click(screen.getByRole('button', { name: 'Open System Settings' }))
    expect(invoked('open_notification_settings')).toHaveLength(1)
  })

  it('offers to ask again when macOS has not asked yet', async () => {
    prime({ ...OFF, almostOut: true }, 'notDetermined', 'granted')
    renderWithQuery(<NotificationsSection />)
    await userEvent.click(await screen.findByRole('button', { name: 'Allow notifications' }))
    expect(invoked('request_notification_permission')).toHaveLength(1)
    await waitFor(() => expect(screen.queryByRole('status')).not.toBeInTheDocument())
  })
})
