import type { AppId } from './app-registry'

export type { AppId } from './app-registry'

export type Surfaces = {
  gui: boolean
  cli: boolean
}

export type Profile = {
  id: string
  app: AppId
  name: string
  slug: string
  color: string
  createdAt: string
  surfaces: Surfaces
  /**
   * Whether the desktop launcher is a wrapper app with a Dock identity of its
   * own (icon, label, pinnable tile) rather than a script that opens the stock
   * app. Off for profiles saved before the setting existed.
   */
  distinctDockIcon: boolean
  /**
   * Whether the desktop app runs from a signed clone: a copy of the stock app
   * kept with the profile, signature untouched, told apart in the Dock by a
   * custom icon. Never together with `distinctDockIcon`. Absent from profiles
   * an older backend lists, which means off.
   */
  signedClone?: boolean
  /**
   * RFC 3339 timestamp of the last `launched_gui` or `copied_cli` event,
   * or `null` if this profile has never been used.
   */
  lastUsedAt: string | null
}

export type DefaultEntry = {
  id: string
  app: AppId
  /** The custom name when the user gave one, else the app's display name. */
  name: string
  /** The name the user gave this entry, or `null` for the stock label. */
  customName: string | null
  surfaces: Surfaces
}

export type SidebarEntry = { kind: 'managed'; profile: Profile } | { kind: 'default'; entry: DefaultEntry }

export type Surface = 'gui' | 'cli'

export type ProfilePatch = {
  name?: string
  color?: string
  /**
   * Switching this rebuilds the launcher in the other shape.
   */
  distinctDockIcon?: boolean
  /**
   * Switching this rebuilds the launcher, and makes or drops the clone.
   */
  signedClone?: boolean
}

/**
 * Why a profile's own Dock-icon launcher was skipped for one launch.
 */
export type WrapperBypass = {
  /**
   * A sentence on what went wrong with the launcher.
   */
  reason: string
}

/**
 * What opening a profile's desktop app came to.
 */
export type LaunchResult = {
  /**
   * The profile, with its last-used time stamped.
   */
  profile: Profile
  /**
   * Set when the profile asks for a launcher of its own that was left out of
   * this launch, with why. The setting itself is untouched.
   */
  wrapperBypass: WrapperBypass | null
}

export type ProfilePaths = {
  dataDir: string
  guiDataDir: string
  cliConfigDir: string
  guiLauncherPath: string | null
  cliWrapperPath: string | null
}

export type ExistingInstallInfo = {
  guiPath: string | null
  cliPath: string | null
  /**
   * Bytes on disk for each detected install. `null` when the corresponding
   * path is also `null` (nothing detected) OR when the size walk hasn't
   * run yet — the boot-time `detect_existing_install` IPC returns `null`
   * here to keep startup fast; sizes arrive later via `detect_existing_sizes`.
   * Permission-denied subpaths during the walk are silently skipped on the
   * Rust side, so the eventual value is best-effort.
   */
  guiSizeBytes: number | null
  cliSizeBytes: number | null
}

export type ExistingInstallSizes = {
  guiSizeBytes: number | null
  cliSizeBytes: number | null
}

export type ImportExistingInput = {
  name: string
  color: string
  includeGui: boolean
  includeCli: boolean
}

export type MigrationBackupInfo = {
  path: string
  createdAtMs: number
  sizeBytes: number
  eligibleForCleanup: boolean
}

export type AppDependency = {
  guiInstalled: boolean
  cliInstalled: boolean
}

export type Dependencies = {
  apps: Record<AppId, AppDependency>
  localBinOnPath: boolean
}

export type Shell = 'zsh' | 'bash' | 'fish'

export type PathHookOutcome =
  | { outcome: 'alreadyInstalled'; rcPath: string }
  | { outcome: 'installed'; rcPath: string; backupPath: string }

export type ThemeMode = 'light' | 'system' | 'dark'

export type AppState = {
  welcomeShown: boolean
  migrationDismissedAt: string | null
  pathBannerDismissedAt: string | null
  themeMode: ThemeMode
  selectedEntryId: string | null
  /**
   * When the user first confirmed they understand what giving a profile its own
   * Dock icon involves. `null` until then, which is when the explanation is shown.
   */
  dockIconAcknowledgedAt: string | null
  /** Names the user gave the stock-install entries. Absent key → stock label. */
  defaultProfileNames: Partial<Record<AppId, string>>
  /**
   * The sessions the user said not now to repairing, by profile id: the ones
   * that needed repair when the offer was dismissed. It stays away until a
   * session not among them needs repair.
   */
  dismissedRepairSessions: Record<string, Array<string>>
  /**
   * The entries the user took out of the menu bar's strip, by id: a managed
   * profile's id, or a stock install's `default:<app>`. The panel lists them
   * all the same.
   */
  hiddenFromMenuBar: Array<string>
  /**
   * The menu bar's, its panel's and their features' settings.
   */
  settings: Settings
}

/**
 * Whether macOS lets claude-switch post notifications. `unavailable` from a build that runs outside
 * an app bundle, which can't post any.
 */
export type NotificationPermission = 'notDetermined' | 'denied' | 'granted' | 'unavailable'

export type AppStatePatch = {
  welcomeShown?: boolean
  migrationDismissedAt?: string
  pathBannerDismissedAt?: string
  themeMode?: ThemeMode
  clearMigrationDismissed?: boolean
  clearPathBannerDismissed?: boolean
  selectedEntryId?: string | null
  clearSelectedEntryId?: boolean
  /**
   * Records the acknowledgement. It cannot be taken back.
   */
  dockIconAcknowledgedAt?: string
  /**
   * Renames one app's stock-install entry. An empty name restores the stock label.
   */
  defaultProfileName?: { app: AppId; name: string }
  /**
   * Sets the sessions a profile's repair offer was dismissed for. An empty
   * list forgets the dismissal.
   */
  dismissedRepair?: { profileId: string; sessionIds: Array<string> }
  /**
   * Shows one entry in the menu bar's strip, or takes it out.
   */
  menuBar?: { id: string; shown: boolean }
  /**
   * Replaces the settings groups it carries.
   */
  settings?: Partial<Settings>
}

/**
 * What the About dialog renders. Sourced from `Cargo.toml` via Cargo's
 * `env!` macros on the Rust side, so editing the manifest (e.g. setting
 * `repository = "https://github.com/…"`) automatically populates the
 * dialog on next build.
 */
export type AppMetadata = {
  name: string
  version: string
  description: string
  authors: Array<string>
  repository: string | null
  homepage: string | null
  license: string | null
}

export type UsageWindow = {
  utilization: number | null
  resetsAt: string | null
  windowDurationMins?: number | null
  /**
   * Server-supplied display name for a window the client can't label on its
   * own — the model a scoped weekly quota applies to, e.g. `Fable`. Absent
   * for windows whose label is fixed copy.
   */
  label?: string | null
}

export type RateLimitResetCredits = {
  availableCount: number
  credits: Array<{ title: string | null; status: string; expiresAt: number | null }> | null
}

/**
 * Pay-as-you-go credit spend. Amounts stay in the currency's minor units
 * (pence, cents) exactly as the backend reports them, so no rounding
 * happens before the formatter sees them.
 */
export type Spend = {
  /**
   * Amount consumed this period, in minor units.
   */
  usedMinor: number
  /**
   * ISO-4217 code the amounts are denominated in, e.g. `GBP`.
   */
  currency: string
  /**
   * Decimal places the minor units carry — 2 for `GBP`, 0 for `JPY`.
   */
  exponent: number
  /**
   * Spend cap in minor units. Null for an uncapped account.
   */
  limitMinor: number | null
  /**
   * Server-computed share of the cap consumed, on a 0..=100 scale.
   */
  percent: number | null
}

export type QuotaUsage = {
  primary: UsageWindow | null
  secondary: UsageWindow | null
  /**
   * Weekly sub-quotas scoped to a single model, each carrying its own
   * `label`. Empty for apps that have none.
   */
  scopedWeekly: Array<UsageWindow>
  rateLimitResetCredits?: RateLimitResetCredits
  spend?: Spend
}

export type QuotaError =
  | 'no_credentials'
  | 'unauthorized'
  | 'forbidden'
  | 'needs_login'
  | 'rate_limited'
  | 'network'
  | 'unknown'

export type ProfileUsage = {
  quota: QuotaUsage | null
  quotaError: QuotaError | null
  fetchedAt: string
}

/**
 * A profile the menu bar reports on, as of its last usage fetch.
 */
export type TrayEntry = {
  id: string
  app: AppId
  name: string
  /**
   * The CLI command the profile signs in with, e.g. `claude-work`.
   */
  command: string
  /**
   * The account's plan as its app names it, e.g. `Max 20x`, when known.
   */
  plan: string | null
  /**
   * The last usage that came back with a quota, kept through failed fetches.
   */
  usage: ProfileUsage | null
  /**
   * What that usage and the profile's local history report.
   */
  metrics: Array<Metric>
  /**
   * Why the latest fetch brought no quota, if it didn't.
   */
  error: QuotaError | null
  /**
   * While a rate limit holds the profile back, when it is asked again.
   */
  retryAt?: string
}

/**
 * What using a Codex rate-limit reset did.
 */
export type ResetOutcome = 'reset' | 'nothingToReset' | 'noCredit' | 'failed'

/**
 * Whether claude-switch opens at login, as macOS has it. `needsApproval`:
 * turned on, waiting to be allowed in System Settings. `unavailable`: this
 * copy can't be a login item (a development build).
 */
export type LoginItem = 'on' | 'off' | 'needsApproval' | 'unavailable'

/**
 * A model Codex usage without a price of its own can be estimated at.
 */
export type FallbackModel = {
  id: string
  title: string
}

/**
 * Whether the menu bar's fetch is running, and when the next is due.
 */
export type TrayStatus = {
  refreshing: boolean
  nextRefreshAt: string | null
}

export type MetricUnit =
  | { type: 'percent' }
  | { type: 'count' }
  | { type: 'money'; currency: string }
  | { type: 'tokens' }

/**
 * One model's share of a period's spend, in USD to the cent. For `Other`, `variants` lists the models folded into it.
 */
type ModelSpend = {
  model: string
  tokens: number
  cost: number
  variants?: Array<ModelSpend>
}

/**
 * What spend estimated from the local logs is made of.
 */
type SpendBreakdown = {
  /**
   * Most expensive first; models under 5% of the period, and any past the fifth, fold into `Other`.
   */
  models: Array<ModelSpend>
  /**
   * Models no price list knows. Their tokens aren't counted, unless priced at a fallback model's rates.
   */
  unknownModels?: Array<string>
  /**
   * The fallback models whose rates priced part of the period.
   */
  fallbackModels?: Array<string>
}

/**
 * How a meter is faring, worked out by the backend when the entries were sent.
 * Every state keys off the share used, whichever way the meter is shown.
 */
type MeterState =
  | { kind: 'spent' }
  | { kind: 'runningOut'; runsOutAt: string | null; projectedFraction: number }
  | { kind: 'closeToLimit'; sparePercent: number; projectedFraction: number }
  | { kind: 'healthy'; projectedFraction: number }
  | { kind: 'level'; severity: 'normal' | 'warning' | 'critical' }

type MetricBase = {
  /**
   * Stable: what stars and layouts refer to, e.g. `session`, `weekly.fable`.
   */
  id: string
  label: string
  state?: MeterState
}

export type Metric =
  | (MetricBase & {
      kind: 'meter'
      used: number
      limit: number
      unit: MetricUnit
      resetsAt: string | null
      periodSecs: number | null
      /**
       * The window opens with the first message, and none has been sent.
       */
      notStarted: boolean
    })
  | (MetricBase & {
      kind: 'values'
      values: Array<{ amount: number; unit: MetricUnit }>
      /**
       * When each counted item lapses, soonest first.
       */
      expiresAt?: Array<string>
      /**
       * Of spend read from the local logs (`today`, `yesterday`, `last30`).
       */
      breakdown?: SpendBreakdown
    })
  | (MetricBase & {
      kind: 'chart'
      unit: MetricUnit
      /**
       * A reading per local day (`YYYY-MM-DD`), oldest first.
       */
      points: Array<{ date: string; value: number }>
    })

/**
 * The `claude-switch` command Settings installs, as `command_line` reports it.
 */
export type CommandLine = {
  /**
   * Where it is, or would be, installed, `~` for the home directory.
   */
  path: string
  /**
   * `conflict`: something that isn't claude-switch's has the name.
   */
  state: 'notInstalled' | 'installed' | 'conflict'
}

/**
 * Mirrors `Settings` in src-tauri/src/settings.rs; one group per Settings section.
 */
export type Settings = {
  menuBar: {
    style: 'text' | 'bars'
    /**
     * Starred metric ids by entry id. An entry without one shows session and weekly.
     */
    stars: Record<string, Array<string>>
    shortcut: string | null
  }
  display: {
    meterStyle: 'used' | 'left'
    resetDisplay: 'countdown' | 'exact'
    alwaysShowPacing: boolean
    timeFormat: 'auto' | '12h' | '24h'
    density: 'default' | 'compact'
    reduceAnimations: boolean
    increaseTransparency: boolean
    totalSpend: {
      shown: boolean
      metric: 'cost' | 'costPerMillionTokens' | 'tokens'
      period: 'today' | 'yesterday' | 'last30Days'
    }
  }
  layout: {
    /**
     * By entry id. An entry without one shows its app's default layout.
     */
    profiles: Record<string, { alwaysVisible: Array<string>; onDemand: Array<string> }>
    expanded: Array<string>
  }
  notifications: { almostOut: boolean; cuttingItClose: boolean; willRunOut: boolean }
  privacy: { hideWhileScreenSharing: boolean }
  integrations: { localApi: boolean }
  sync: { icloud: boolean }
  updates: { checkAutomatically: boolean; beta: boolean }
  advanced: { logLevel: 'error' | 'warning' | 'info' | 'debug'; codexFallbackModel: string | null }
}

/** The account a profile is signed in under, from what its CLI keeps on disk. */
export type ProfileAccount = {
  email: string | null
  /** The person's name, as the app recorded it. */
  name: string | null
  organization: string | null
  /** The subscription, e.g. "Max" or "Pro". */
  plan: string | null
}

/**
 * Whether a profile is signed in, and as whom. `unknown` is signed in, or may
 * be, as an account nothing on disk names: a Claude desktop app signed in as
 * someone its `.claude.json` doesn't mention.
 */
export type AccountStatus =
  | { status: 'signedIn'; account: ProfileAccount }
  | { status: 'signedOut' }
  | { status: 'unknown' }

/**
 * Where a session was started: the desktop app (Claude's Code tab, Codex
 * desktop) or the CLI (including IDE extensions).
 */
export type SessionKind = 'desktop' | 'cli'

/**
 * What a session's files are doing right now. `transcriptMissing` is a
 * desktop record whose transcript was cleaned up.
 */
export type SessionState = 'idle' | 'openInTerminal' | 'openInDesktop' | 'transcriptMissing'

/**
 * One coding session a profile owns, as its row shows it: a Claude Code
 * session (CLI or the desktop app's Code tab) or a Codex thread.
 */
export type Session = {
  /**
   * Claude: the id of the transcript shown, which for a desktop session is its
   * record's `cliSessionId`, else the last used of its `priorCliSessionIds`. A
   * desktop session whose transcripts are all gone keeps its `cliSessionId`,
   * else its record's `local_<uuid>`. Codex: the thread id.
   */
  id: string
  /**
   * Where the session was started.
   */
  kind: SessionKind
  /**
   * The desktop title, else the `/rename` name, else the generated title,
   * else the first prompt. Null when none of those says anything.
   */
  title: string | null
  /**
   * The folder the session works in.
   */
  cwd: string | null
  /**
   * The last thing typed into the session.
   */
  lastPrompt: string | null
  /**
   * When the session was last used, ISO 8601.
   */
  lastUsedAt: string
  /**
   * The session is archived.
   */
  archived: boolean
  /**
   * What the session's files are doing right now.
   */
  state: SessionState
  /**
   * One of the session's transcripts sits in another profile's config dir and
   * should be moved into this one's.
   */
  needsRepair: boolean
  /**
   * Why the session can't be moved to another profile, if it can't.
   */
  unmovableReason: string | null
}

/**
 * What listing a profile's sessions returns: every session it owns, and how
 * many of them need repair.
 */
export type SessionList = {
  /**
   * Active and archived, most recently used first.
   */
  sessions: Array<Session>
  /**
   * How many of the sessions need repair.
   */
  repairCount: number
}

/**
 * What can be done to a session: put it in the Archived tab, or bring it back.
 */
export type SessionAction = 'archive' | 'restore'

/**
 * A desktop app instance that has to quit before a session action can run.
 */
export type AppToQuit = {
  /**
   * The profile (or `default:<app>`) whose instance it is.
   */
  homeId: string
  /**
   * How the instance is named: `Claude (Work)`.
   */
  label: string
}

/**
 * What stands between a session and an action.
 */
export type ActionCheck = {
  /**
   * Why the action can't run, when only the user can change that.
   */
  blocker: string | null
  /**
   * The desktop app that has to quit first, when it holds files the action
   * writes and is running.
   */
  appToQuit: AppToQuit | null
}

/**
 * What a move does with one of the session's files or folders at the
 * destination: copies it there, leaves the same one there alone, or backs up
 * a different one there and replaces it.
 */
export type ItemAction = 'copy' | 'same' | 'replace'

/**
 * What a move does about the destination's desktop app: lists the session
 * there, finds it listed already, can't as the app isn't signed in, or can't
 * as the destination has no desktop app.
 */
export type DesktopAction = 'add' | 'alreadyListed' | 'signInNeeded' | 'noDesktop'

/**
 * One file or folder a move copies.
 */
export type PlannedItem = {
  /**
   * Where it goes, relative to the destination's config dir.
   */
  path: string
  /**
   * What the move does with it.
   */
  action: ItemAction
}

/**
 * What moving a session to another profile would do.
 */
export type MovePlan = {
  /**
   * One line saying what moves where: `Moves 3 files from Work to Personal,
   * and 2 memory files`.
   */
  summary: string
  /**
   * The files and folders the move copies, then the project memory files it
   * copies, then the transcripts.
   */
  items: Array<PlannedItem>
  /**
   * The destination has a copy that was used more recently, which the move
   * only replaces when the user agrees.
   */
  destinationNewer: boolean
  /**
   * What the move does about the destination's desktop app.
   */
  desktop: DesktopAction
  /**
   * Why the move can't be done, when only the user can change that.
   */
  blockers: Array<string>
  /**
   * The desktop apps that have to quit first, at the source, the destination
   * or both.
   */
  appsToQuit: Array<AppToQuit>
  /**
   * Things worth knowing that don't stop the move.
   */
  notes: Array<string>
}

/**
 * What a move did that the user should hear about.
 */
export type MoveReport = {
  /**
   * The memory files both profiles have, differently; the destination's were
   * kept.
   */
  memoryConflicts: Array<string>
}

/**
 * A session a repair left as it was, and why.
 */
export type SkippedSession = {
  /**
   * The session's id.
   */
  id: string
  /**
   * Why it was left as it was.
   */
  reason: string
}

/**
 * What repairing a profile's sessions did.
 */
export type RepairReport = {
  /**
   * How many sessions were repaired.
   */
  repaired: number
  /**
   * The sessions that needed repair but were left as they were.
   */
  skipped: Array<SkippedSession>
  /**
   * The memory files both folders have, differently; the profile's own were
   * kept.
   */
  memoryConflicts: Array<string>
  /**
   * What the repair did that it didn't mean to, such as a copy it left
   * behind, for the user to tidy.
   */
  warnings: Array<string>
}
