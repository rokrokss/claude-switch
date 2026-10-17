# Changelog

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
