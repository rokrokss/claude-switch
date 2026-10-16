import type { ReactElement } from 'react'

import { invoke } from '@tauri-apps/api/core'
import { screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { ThemeProvider, ToastProvider } from '@/design'
import { defaultSettings } from '@/lib/settings'
import { renderWithQuery } from '@/test/render-with-query'

import { SettingsView } from './settings-view'

function renderSettings(ui: ReactElement) {
  return renderWithQuery(
    <ThemeProvider defaultMode="system">
      <ToastProvider>{ui}</ToastProvider>
    </ThemeProvider>,
  )
}

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn().mockResolvedValue('0.1.0') }))

const mockInvoke = vi.mocked(invoke)

const HEALTHY_DEPS = {
  apps: {
    claude: { guiInstalled: true, cliInstalled: true },
    codex: { guiInstalled: false, cliInstalled: false },
  },
  localBinOnPath: true,
}

const DEFAULT_STATE = {
  welcomeShown: true,
  migrationDismissedAt: null,
  pathBannerDismissedAt: null,
  themeMode: 'system' as const,
  settings: defaultSettings,
}

const trayEntries = [
  {
    id: 'p1',
    app: 'claude',
    name: 'Work',
    command: 'claude-work',
    plan: null,
    usage: null,
    metrics: [
      {
        id: 'session',
        label: 'Session',
        kind: 'meter',
        used: 1,
        limit: 100,
        unit: { type: 'percent' },
        resetsAt: null,
        periodSecs: null,
        notStarted: false,
      },
      {
        id: 'weekly',
        label: 'Weekly',
        kind: 'meter',
        used: 1,
        limit: 100,
        unit: { type: 'percent' },
        resetsAt: null,
        periodSecs: null,
        notStarted: false,
      },
      { id: 'resets', label: 'Rate limit resets', kind: 'values', values: [{ amount: 2, unit: { type: 'count' } }] },
    ],
    error: null,
  },
]

const DEFAULT_EXISTING = {
  guiPath: null,
  cliPath: null,
  guiSizeBytes: null,
  cliSizeBytes: null,
}

const DEFAULT_METADATA = {
  name: 'claude-switch',
  version: '0.1.0',
  description: 'Test description',
  authors: ['Hyungrok Kim'],
  repository: null,
  homepage: null,
  license: 'MIT',
}

beforeEach(() => {
  mockInvoke.mockReset()
})

// NOTE: If a future change adds a new hook to SettingsView (or a new IPC call
// during the initial render), update primeInitialLoads to dispatch the new
// command name. The dispatch-by-name pattern avoids ordering coupling.
function primeInitialLoads({
  deps = HEALTHY_DEPS,
  backups = [] as Array<unknown>,
  state = DEFAULT_STATE,
  shell = 'zsh',
  existing = DEFAULT_EXISTING,
  metadata = DEFAULT_METADATA,
}: {
  deps?: unknown
  backups?: Array<unknown>
  state?: unknown
  shell?: string
  existing?: unknown
  metadata?: unknown
} = {}) {
  const answers: Record<string, (args: unknown) => unknown> = {
    check_dependencies: () => deps,
    list_migration_backups: () => backups,
    load_app_state: () => state,
    detect_shell: () => shell,
    // `existing` describes the Claude stock install under test; Codex
    // detects nothing so exactly one Re-import card renders.
    detect_existing_install: (args) =>
      (args as { app?: string } | undefined)?.app === 'codex' ? DEFAULT_EXISTING : existing,
    get_app_metadata: () => metadata,
    login_item: () => 'off',
    update_app_state: () => state,
    set_login_item: (args) => ((args as { enabled: boolean }).enabled ? 'on' : 'off'),
    tray_usage: () => trayEntries,
    command_line: () => ({ path: '~/.local/bin/claude-switch', state: 'notInstalled' }),
    codex_fallback_models: () => [{ id: 'gpt-5.5', title: 'GPT 5.5' }],
    refresh_usage: () => undefined,
  }
  mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
    const answer = answers[command]
    if (!answer) {
      throw new Error(`unexpected command in test: ${command}`)
    }
    return answer(args)
  })
}

describe('SettingsView', () => {
  it('moves a metric on demand, and undoes it', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    const user = userEvent.setup()
    const where = await screen.findByRole('radiogroup', { name: 'Where Session shows' })
    await user.click(within(where).getByText('On demand'))
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('update_app_state', {
        patch: {
          settings: {
            layout: {
              ...defaultSettings.layout,
              profiles: { p1: { alwaysVisible: ['weekly'], onDemand: ['resets', 'session'] } },
            },
            menuBar: defaultSettings.menuBar,
          },
        },
      }),
    )
    await user.click(screen.getByRole('button', { name: 'Undo' }))
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith('update_app_state', {
        patch: { settings: { layout: defaultSettings.layout, menuBar: defaultSettings.menuBar } },
      }),
    )
  })

  it('refuses a third star for the menu bar', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'Star Rate limit resets for the menu bar' }))
    expect(await screen.findByText('Up to 2 stars per profile')).toBeInTheDocument()
  })

  it('saves a usage display setting as its whole group', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('switch', { name: 'Always show pacing' }))
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('update_app_state', {
        patch: { settings: { display: { ...defaultSettings.display, alwaysShowPacing: true } } },
      }),
    )
  })

  it('turns opening at login on through macOS', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    const user = userEvent.setup()
    const login = await screen.findByRole('switch', { name: 'Open at login' })
    await waitFor(() => expect(login).toBeEnabled())
    await user.click(login)
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('set_login_item', { enabled: true }))
    await waitFor(() => expect(login).toHaveAttribute('aria-checked', 'true'))
  })

  it('prices unknown Codex models at a chosen model, then works spend out again', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('combobox', { name: 'Price unknown Codex models as' }))
    await user.click(await screen.findByRole('option', { name: 'GPT 5.5' }))
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('update_app_state', {
        patch: { settings: { advanced: { ...defaultSettings.advanced, codexFallbackModel: 'gpt-5.5' } } },
      }),
    )
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('refresh_usage'))
  })

  it('records a shortcut for the panel', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    const user = userEvent.setup()
    await user.click(await screen.findByRole('button', { name: 'Record shortcut' }))
    await user.keyboard('{Meta>}{Shift>}u{/Shift}{/Meta}')
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('update_app_state', {
        patch: { settings: { menuBar: { ...defaultSettings.menuBar, shortcut: 'Command+Shift+U' } } },
      }),
    )
  })

  it('renders the app version in the footer row', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    expect(await screen.findByText(/claude-switch v0\.1\.0/)).toBeInTheDocument()
  })

  it('renders the System status card with one row per dependency', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    await waitFor(() => expect(screen.getByText('Claude Desktop')).toBeInTheDocument())
    expect(screen.getByText('Claude CLI')).toBeInTheDocument()
    expect(screen.getByText('ChatGPT Desktop')).toBeInTheDocument()
    expect(screen.getByText('Codex CLI')).toBeInTheDocument()
    expect(screen.getByText('Shell PATH')).toBeInTheDocument()
  })

  it('shows the no-backups empty card when there are none', async () => {
    primeInitialLoads({ backups: [] })
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    expect(await screen.findByText(/No migration backups/)).toBeInTheDocument()
  })

  it('renders the Re-import action card when an install is detected', async () => {
    primeInitialLoads({
      existing: {
        guiPath: '/Users/me/Library/Application Support/Claude',
        cliPath: null,
        guiSizeBytes: 248 * 1024 * 1024,
        cliSizeBytes: null,
      },
    })
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    expect(await screen.findByText(/Detected an existing Claude install/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Re-import/ })).toBeInTheDocument()
  })

  it('hides the Re-import action card when nothing is detected', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    await waitFor(() => expect(screen.getByText('Claude Desktop')).toBeInTheDocument())
    expect(screen.queryByText(/Detected an existing Claude install/)).not.toBeInTheDocument()
  })

  it('Re-import button calls onOpenMigration', async () => {
    primeInitialLoads({
      existing: {
        guiPath: '/Users/me/Library/Application Support/Claude',
        cliPath: null,
        guiSizeBytes: null,
        cliSizeBytes: null,
      },
    })
    const onOpenMigration = vi.fn()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={onOpenMigration} onOpenAbout={vi.fn()} />)
    await userEvent.setup().click(await screen.findByRole('button', { name: /Re-import/ }))
    expect(onOpenMigration).toHaveBeenCalledWith('claude')
  })

  it('shows a confirmation dialog before resetting onboarding flags', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    await waitFor(() => expect(screen.getByText('Claude Desktop')).toBeInTheDocument())

    mockInvoke.mockReset()
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'update_app_state') {
        return DEFAULT_STATE
      }
      throw new Error(`unexpected command in test: ${command}`)
    })

    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: /Reset onboarding flags/ }))

    expect(screen.getByRole('dialog', { name: /Reset onboarding state\?/ })).toBeInTheDocument()
    expect(mockInvoke).not.toHaveBeenCalled()

    await user.click(screen.getByRole('button', { name: 'Reset' }))

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('update_app_state', {
        patch: {
          welcomeShown: false,
          clearMigrationDismissed: true,
          clearPathBannerDismissed: true,
        },
      }),
    )
  })

  it('cancels reset when the user picks Cancel', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    await waitFor(() => expect(screen.getByText('Claude Desktop')).toBeInTheDocument())

    mockInvoke.mockReset()
    mockInvoke.mockImplementation(async (command: string) => {
      throw new Error(`unexpected command in test: ${command}`)
    })

    const user = userEvent.setup()
    await user.click(screen.getByRole('button', { name: /Reset onboarding flags/ }))
    await user.click(screen.getByRole('button', { name: 'Cancel' }))

    expect(mockInvoke).not.toHaveBeenCalled()
    expect(screen.queryByRole('dialog', { name: /Reset onboarding state\?/ })).not.toBeInTheDocument()
  })

  it('appearance segmented control persists the theme via update_app_state', async () => {
    primeInitialLoads()
    renderSettings(<SettingsView onClose={vi.fn()} onOpenMigration={vi.fn()} onOpenAbout={vi.fn()} />)
    await waitFor(() => expect(screen.getByText('Claude Desktop')).toBeInTheDocument())

    mockInvoke.mockReset()
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'update_app_state') {
        return { ...DEFAULT_STATE, themeMode: 'dark' }
      }
      throw new Error(`unexpected command in test: ${command}`)
    })

    await userEvent.setup().click(screen.getByRole('radio', { name: /Dark theme/ }))

    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('update_app_state', { patch: { themeMode: 'dark' } }))
    // The Dark radio is checked after the click — the segmented control
    // is the only source of truth for the chosen mode now that the
    // "Currently: …" helper line is gone.
    expect(screen.getByRole('radio', { name: /Dark theme/ })).toHaveAttribute('aria-checked', 'true')
  })
})
