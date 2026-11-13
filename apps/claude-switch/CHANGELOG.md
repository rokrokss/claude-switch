# Changelog

## 2.3.2 (2026-11-14)


### Fixed

* **usage:** figures that stopped updating, after a sign-in lapsed or while a rate limit holds the profile, show as outdated instead of as current: greyed, without a pace mark, with when they were read ("read 6h ago") in the panel and on the profile's usage card, and faded in the menu bar
* **usage:** while a rate limit holds a Claude profile back, its Fable row follows the reading Claude Code gets with each reply, as its 5-hour and weekly rows already did; when that reading has none, the row says when it was last read

## 2.3.1 (2026-11-13)


### Fixed

* **usage:** a Claude profile's usage card, the menu bar and `claude-switch usage` no longer show another account's quota when the profile's stored sign-in belongs to a different account; the card names that account and offers to sign in again
* **usage:** a Claude sign-in that lapsed after renewing it failed is renewed again, instead of reading as rate limited for up to an hour
* **usage:** a Claude profile's Keychain sign-in is read before a `.credentials.json` Claude Code left behind, so usage no longer goes out on a sign-in long since renewed
* **usage:** spend prices Claude Sonnet 5.5's cache reads at $0.10 instead of $0.20, and Claude Haiku 5.5's prompts past 100k tokens at its long-prompt rates
* **shortcuts:** ⌘⌫ and ⌘C in a text field delete and copy text, instead of opening the delete dialog or copying the profile's CLI command

## 2.3.0 (2026-10-22)


### Added

* **profiles:** Antigravity profiles: an `agy-<slug>` command runs Google's Antigravity CLI with the profile as its home and a sign-in of its own, and with the rest of your home and your global rules (`~/.gemini/GEMINI.md`) as they are. CLI only. Google moved its individual plans (free, AI Pro, AI Ultra) from Gemini CLI to Antigravity on June 18
* **usage:** Antigravity usage on the profile and in the menu bar: Gemini's 5-hour and weekly limits, and each other model group's (Claude and GPT models, …), read through `agy -p /usage`, which starts no model turn

### Fixed

* **migration:** importing `~/.gemini` as a Gemini profile leaves Antigravity's folders in it (`antigravity*`, `config`) where they are, instead of moving them into the profile and the backup

## 2.2.0 (2026-10-21)


### Added

* **profiles:** Gemini profiles: a `gemini-<slug>` command in `~/.local/bin` runs Gemini CLI with the profile's own sign-in, settings and history (`GEMINI_CLI_HOME`), and inherits your skills, agents, commands and `GEMINI.md` from `~/.gemini` and `~/.agents`. CLI only, since Gemini.app can't run per profile; usage, spend and sessions aren't shown for Gemini yet
* **profiles:** an existing `~/.gemini` shows as Gemini's default entry and can be imported as a profile

## 2.1.1 (2026-10-17)


### Fixed

* **tray:** the panel's footer says when the next update comes, instead of always reading "Updating…"

## 2.1.0 (2026-10-17)


### Added

* **usage:** a Claude profile the usage endpoint keeps rate limiting still shows a fresh session and weekly, read through Claude Code at most every 15 minutes while it waits

### Fixed

* **usage:** a rate limit that asks for a wait longer than an hour is waited out in full, instead of being asked again early
* **usage:** a ChatGPT profile whose usage can't be read says why in the log, such as codex missing from PATH

## 2.0.2 (2026-10-17)


### Fixed

* **settings:** Open at login can be turned on in a copy that has never been a login item, instead of reading as unavailable

## 2.0.1 (2026-10-17)


### Added

* **menu bar:** claude-switch stays in the menu bar with every Claude and ChatGPT profile's usage at a glance: values or bars, stars for what it shows, hidden while the screen is shared, and a shortcut to its panel
* **panel:** a click on the item opens each profile's card with its session, weekly, extra usage, credits and resets, pacing and when a window runs out, plus a total spend card and a usage trend
* **usage:** spend and tokens for today, yesterday and the last 30 days, worked out on this Mac from each profile's session logs, by model
* **usage:** Claude sign-ins renew themselves; ChatGPT plans, credits and rate-limit resets show, and a reset can be used from the panel
* **notifications:** a notification when a limit runs low, is close, or will run out before it resets
* **sync:** usage history adds up across your Macs through iCloud (Settings → Sync)
* **integrations:** `claude-switch usage` prints the limits as JSON, an optional local API serves them, and requests can go through a proxy
* **settings:** menu bar, usage display, panel layout, notifications, login item, log level and Quit, all in Settings

### Changed

* **design:** a lighter Liquid Glass look, in light and dark
* **window:** closing the window keeps claude-switch in the menu bar, without its Dock icon; quit from the menu bar, Settings or ⌘Q
* **usage:** a rate-limited profile isn't asked again until its wait is over, even after a restart, and its card says when

## 2.0.0 (2026-10-16)


### Added

* **updater:** install updates from the app; releases are signed with claude-switch's own key (1.4.0 can't verify them, so it is updated by hand once)

### Changed

* **design:** take on shizue's theme: a cream canvas and mint actions in light, deep green in dark, following macOS or the choice in Settings → Appearance
* **design:** a new pixel mark for the app icon, the sidebar and the favicon, beside the claude-switch wordmark, in place of the ai-profiles logo and name
* **profiles:** a profile's header shows its app's logo with a badge in the profile's color, as its Dock tile does
* **usage:** say that Claude Code signs in apart from the desktop app when a profile shows no usage

### Fixed

* **launchers:** bring a signed clone's Google sign-in back to its own window
* **launchers:** bring a script-mode profile's Google sign-in back to its own window too

## 1.4.0 (2026-10-14)


### Added

* **launchers:** open Claude Desktop profiles from a signed clone of Claude.app that keeps Anthropic's signature, with a Dock icon badged in the profile's color
* **launchers:** reopen a signed clone that macOS started without its profile, after an update or from the Dock
* **profiles:** offer the signed clone option for Claude profiles

### Changed

* name the app, its bundle identifiers and its data folder claude-switch
