import type { AppId } from './app-registry'
import type {
  AccountStatus,
  ActionCheck,
  AppMetadata,
  AppState,
  AppStatePatch,
  CommandLine,
  Dependencies,
  ExistingInstallInfo,
  ExistingInstallSizes,
  ImportExistingInput,
  LaunchResult,
  LoginItem,
  MigrationBackupInfo,
  MovePlan,
  MoveReport,
  NotificationPermission,
  PathHookOutcome,
  Profile,
  ProfilePatch,
  ProfilePaths,
  ProfileUsage,
  RepairReport,
  ResetOutcome,
  SessionAction,
  SessionList,
  Shell,
  Surface,
  Surfaces,
  TrayEntry,
  TrayStatus,
} from './types'

import { invoke } from '@tauri-apps/api/core'
import { writeText } from '@tauri-apps/plugin-clipboard-manager'

export function listProfiles(): Promise<Array<Profile>> {
  return invoke<Array<Profile>>('list_profiles')
}

export function createProfile(input: {
  app: AppId
  name: string
  color: string
  surfaces: Surfaces
  /**
   * Opt-in: leaving it out gives the profile the plain script launcher.
   */
  distinctDockIcon?: boolean
  /**
   * Opt-in, like `distinctDockIcon`, and never together with it.
   */
  signedClone?: boolean
}): Promise<Profile> {
  return invoke<Profile>('create_profile', input)
}

export function updateProfile(input: { id: string; patch: ProfilePatch }): Promise<Profile> {
  return invoke<Profile>('update_profile', input)
}

export function deleteProfile(input: { id: string; moveToTrash: boolean }): Promise<void> {
  return invoke('delete_profile', input)
}

export function reorderProfiles(ids: Array<string>): Promise<Array<Profile>> {
  return invoke<Array<Profile>>('reorder_profiles', { ids })
}

export function toggleSurface(input: { id: string; surface: Surface; enabled: boolean }): Promise<Profile> {
  return invoke<Profile>('toggle_surface', input)
}

/**
 * Opens the profile's desktop app, or focuses it. For a profile whose signed
 * clone has been running a while without the profile, this rejects with kind
 * `StrayRunning` unless `quitStray` says the user agreed to have it quit.
 */
export function openProfileInApp(id: string, quitStray?: boolean): Promise<LaunchResult> {
  return invoke<LaunchResult>('open_profile_in_app', { id, quitStray })
}

export function touchProfileLastUsed(id: string): Promise<Profile> {
  return invoke<Profile>('touch_profile_last_used', { id })
}

export function openInFinder(path: string): Promise<void> {
  return invoke('open_in_finder', { path })
}

export function openDefaultGui(app: AppId, dataDir: string): Promise<void> {
  return invoke('open_default_gui', { app, dataDir })
}

export function profilePaths(id: string): Promise<ProfilePaths> {
  return invoke<ProfilePaths>('profile_paths', { id })
}

export function profileAccount(id: string): Promise<AccountStatus> {
  return invoke<AccountStatus>('profile_account', { id })
}

export function copyToClipboard(text: string): Promise<void> {
  return writeText(text)
}

export function detectExistingInstall(app: AppId): Promise<ExistingInstallInfo> {
  return invoke<ExistingInstallInfo>('detect_existing_install', { app })
}

export function detectExistingSizes(app: AppId): Promise<ExistingInstallSizes> {
  return invoke<ExistingInstallSizes>('detect_existing_sizes', { app })
}

export function importExistingInstall(app: AppId, input: ImportExistingInput): Promise<Profile> {
  return invoke<Profile>('import_existing_install', { app, input })
}

export function listMigrationBackups(): Promise<Array<MigrationBackupInfo>> {
  return invoke<Array<MigrationBackupInfo>>('list_migration_backups')
}

export function deleteMigrationBackup(path: string): Promise<void> {
  return invoke('delete_migration_backup', { path })
}

export function checkDependencies(): Promise<Dependencies> {
  return invoke<Dependencies>('check_dependencies')
}

export function detectShell(): Promise<Shell> {
  return invoke<Shell>('detect_shell')
}

export function installPathHook(shell: Shell): Promise<PathHookOutcome> {
  return invoke<PathHookOutcome>('install_path_hook', { shell })
}

export function checkCommandLine(): Promise<CommandLine> {
  return invoke<CommandLine>('command_line')
}

export function installCommandLine(): Promise<CommandLine> {
  return invoke<CommandLine>('install_command_line')
}

export function uninstallCommandLine(): Promise<CommandLine> {
  return invoke<CommandLine>('uninstall_command_line')
}

export function loadAppState(): Promise<AppState> {
  return invoke<AppState>('load_app_state')
}

export function updateAppState(patch: AppStatePatch): Promise<AppState> {
  return invoke<AppState>('update_app_state', { patch })
}

export function getAppMetadata(): Promise<AppMetadata> {
  return invoke<AppMetadata>('get_app_metadata')
}

export function openExternalUrl(url: string): Promise<void> {
  return invoke('open_external_url', { url })
}

/**
 * Whether macOS lets claude-switch post notifications.
 */
export function notificationPermission(): Promise<NotificationPermission> {
  return invoke<NotificationPermission>('notification_permission')
}

/**
 * Asks the user to let claude-switch post notifications, if they haven't answered yet, and resolves
 * with what they allow now.
 */
export function requestNotificationPermission(): Promise<NotificationPermission> {
  return invoke<NotificationPermission>('request_notification_permission')
}

/**
 * Opens claude-switch's page of System Settings → Notifications.
 */
export function openNotificationSettings(): Promise<void> {
  return invoke('open_notification_settings')
}

export function openCliLogin(id: string): Promise<void> {
  return invoke('open_cli_login', { id })
}

export function getProfileUsage(profileId: string): Promise<ProfileUsage> {
  return invoke<ProfileUsage>('get_profile_usage', { profileId })
}

/**
 * The profiles the menu bar reports on, as of its last fetch; `null` until
 * the first has finished.
 */
export function getTrayUsage(): Promise<Array<TrayEntry> | null> {
  return invoke<Array<TrayEntry> | null>('tray_usage')
}

export function openMainWindow(): Promise<void> {
  return invoke('open_main_window')
}

/**
 * Whether the menu bar's fetch is running, and when the next is due.
 */
export function getTrayStatus(): Promise<TrayStatus> {
  return invoke<TrayStatus>('tray_status')
}

/**
 * Fetches every profile's usage for the menu bar now.
 */
export function refreshUsage(): Promise<void> {
  return invoke('refresh_usage')
}

/**
 * Uses the Codex profile's rate-limit reset lapsing at `expiresAt` (Unix
 * seconds). It can't be undone.
 */
export function claimCodexReset(profileId: string, expiresAt: number): Promise<ResetOutcome> {
  return invoke<ResetOutcome>('use_codex_reset', { profileId, expiresAt })
}

/**
 * One light trackpad tap, for a dragged row snapping into a slot.
 */
export function hapticTick(): Promise<void> {
  return invoke('haptic_tick')
}

/**
 * Where claude-switch writes its log.
 */
export function getLogFile(): Promise<string | null> {
  return invoke<string | null>('log_file')
}

/**
 * Whether claude-switch opens at login, as macOS has it.
 */
export function getLoginItem(): Promise<LoginItem> {
  return invoke<LoginItem>('login_item')
}

export function setLoginItem(enabled: boolean): Promise<LoginItem> {
  return invoke<LoginItem>('set_login_item', { enabled })
}

/**
 * Hides the menu bar panel.
 */
export function closePanel(): Promise<void> {
  return invoke('close_panel')
}

/**
 * Shows the main window on its Settings pane.
 */
export function openSettings(): Promise<void> {
  return invoke('open_settings')
}

/**
 * Shows the main window with its About dialog.
 */
export function openAbout(): Promise<void> {
  return invoke('open_about')
}

export function quitApp(): Promise<void> {
  return invoke('quit_app')
}

/**
 * The sessions profile `profileId` (or `default:<app>`) owns, active and archived.
 */
export function listSessions(profileId: string): Promise<SessionList> {
  return invoke<SessionList>('list_sessions', { profileId })
}

/**
 * What stands between session `sessionId` of profile `profileId` and `action`:
 * a reason only the user can clear, or the desktop app that has to quit first.
 */
export function checkSessionAction(profileId: string, sessionId: string, action: SessionAction): Promise<ActionCheck> {
  return invoke<ActionCheck>('check_session_action', { profileId, sessionId, action })
}

/**
 * Archives session `sessionId` of profile `profileId`, quitting the desktop app
 * in the way first when `quitApp`.
 */
export function archiveSession(profileId: string, sessionId: string, quitApp: boolean): Promise<void> {
  return invoke('archive_session', { profileId, sessionId, quitApp })
}

/**
 * Restores archived session `sessionId` of profile `profileId`, quitting the
 * desktop app in the way first when `quitApp`.
 */
export function restoreSession(profileId: string, sessionId: string, quitApp: boolean): Promise<void> {
  return invoke('restore_session', { profileId, sessionId, quitApp })
}

/**
 * What moving session `sessionId` of profile `profileId` to profile
 * `destinationId` would do, without doing any of it.
 */
export function planSessionMove(profileId: string, sessionId: string, destinationId: string): Promise<MovePlan> {
  return invoke<MovePlan>('plan_session_move', { profileId, sessionId, destinationId })
}

/**
 * Moves session `sessionId` of profile `profileId` to profile `destinationId`:
 * copies it there, then archives it here. A newer copy there is only replaced
 * if `replaceNewer`; the desktop apps in the way are quit first if `quitApps`.
 */
export function moveSession(
  profileId: string,
  sessionId: string,
  destinationId: string,
  replaceNewer: boolean,
  quitApps: boolean,
): Promise<MoveReport> {
  return invoke<MoveReport>('move_session', { profileId, sessionId, destinationId, replaceNewer, quitApps })
}

/**
 * What stands between the sessions of profile `profileId` that need repair and
 * their repair: the profile's desktop app, when it runs.
 */
export function checkSessionRepair(profileId: string): Promise<ActionCheck> {
  return invoke<ActionCheck>('check_session_repair', { profileId })
}

/**
 * Repairs the sessions profile `profileId`'s desktop app started before the
 * profile had its own folder, quitting that app first if `quitApp`.
 */
export function repairSessions(profileId: string, quitApp: boolean): Promise<RepairReport> {
  return invoke<RepairReport>('repair_sessions', { profileId, quitApp })
}
