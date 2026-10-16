import type { AppState, AppStatePatch, CommandLine } from '@/lib/types'

import { invoke } from '@tauri-apps/api/core'
import { screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { defaultSettings } from '@/lib/settings'
import { renderWithQuery } from '@/test/render-with-query'

import { CommandLineSection } from './command-line-section'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const mockInvoke = vi.mocked(invoke)

const PATH = '~/.local/bin/claude-switch'

/**
 * Answers the section's commands from a little state of its own, so a click
 * that installs or switches something shows on the next render.
 */
function prime(state: CommandLine['state'], localApi = false) {
  let commandLine: CommandLine = { path: PATH, state }
  let appState = {
    welcomeShown: true,
    settings: { ...defaultSettings, integrations: { localApi } },
  } as unknown as AppState
  mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
    if (command === 'command_line') {
      return commandLine
    }
    if (command === 'install_command_line') {
      commandLine = { path: PATH, state: 'installed' }
      return commandLine
    }
    if (command === 'uninstall_command_line') {
      commandLine = { path: PATH, state: 'notInstalled' }
      return commandLine
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

describe('CommandLineSection', () => {
  it('installs the command, then offers to uninstall it', async () => {
    prime('notInstalled')
    renderWithQuery(<CommandLineSection />)
    expect(await screen.findByText(/Installs to ~\/\.local\/bin\/claude-switch/)).toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: 'Install' }))

    expect(mockInvoke).toHaveBeenCalledWith('install_command_line')
    expect(await screen.findByRole('button', { name: 'Uninstall' })).toBeInTheDocument()
    expect(screen.getByText(/Installed at ~\/\.local\/bin\/claude-switch/)).toBeInTheDocument()
  })

  it('uninstalls an installed command', async () => {
    prime('installed')
    renderWithQuery(<CommandLineSection />)

    await userEvent.click(await screen.findByRole('button', { name: 'Uninstall' }))

    expect(mockInvoke).toHaveBeenCalledWith('uninstall_command_line')
    expect(await screen.findByRole('button', { name: 'Install' })).toBeInTheDocument()
  })

  it('leaves a file of someone else alone', async () => {
    prime('conflict')
    renderWithQuery(<CommandLineSection />)
    expect(await screen.findByText(/belongs to something else/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Install' })).toBeDisabled()
  })

  it('says why an install failed', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'command_line') {
        return { path: PATH, state: 'notInstalled' }
      }
      if (command === 'load_app_state') {
        return { settings: defaultSettings }
      }
      if (command === 'install_command_line') {
        throw { kind: 'Io', message: 'Permission denied' }
      }
      throw new Error(`unexpected command in test: ${command}`)
    })
    renderWithQuery(<CommandLineSection />)

    await userEvent.click(await screen.findByRole('button', { name: 'Install' }))

    expect(await screen.findByRole('alert')).toHaveTextContent('Permission denied')
  })

  it('turns the local API on, noting that any web page can read it', async () => {
    prime('notInstalled')
    renderWithQuery(<CommandLineSection />)
    const toggle = await screen.findByRole('checkbox', { name: /Local API/ })
    expect(toggle).toHaveAttribute('aria-checked', 'false')
    expect(screen.getByText('http://127.0.0.1:6741/v1/limits')).toBeInTheDocument()
    expect(screen.getByText(/Any web page open on this Mac can read them/)).toBeInTheDocument()

    await userEvent.click(toggle)

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('update_app_state', {
        patch: { settings: { integrations: { localApi: true } } },
      }),
    )
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'))
  })

  it('turns the local API off', async () => {
    prime('notInstalled', true)
    renderWithQuery(<CommandLineSection />)
    const toggle = await screen.findByRole('checkbox', { name: /Local API/ })
    expect(toggle).toHaveAttribute('aria-checked', 'true')

    await userEvent.click(toggle)

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('update_app_state', {
        patch: { settings: { integrations: { localApi: false } } },
      }),
    )
  })
})
