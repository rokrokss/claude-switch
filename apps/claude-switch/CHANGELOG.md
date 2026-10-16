# Changelog

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
