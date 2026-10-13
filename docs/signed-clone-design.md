# The signed clone launch mode (Claude Desktop on macOS)

- Status: **implemented, runtime verification pending** (2026-10-14)
- Measured on: macOS 26.6.2 (25G83), arm64, Claude Desktop 2.31226.1
- Verification steps: [signed-clone-verification.md](signed-clone-verification.md)

---

## 0. Summary

Each Claude profile gets its own **APFS clone** of `/Applications/Claude.app`, and Claude Desktop runs from that clone. Not a byte inside the clone is changed. Anthropic's signature (CDHash, designated requirement) therefore stays as it is, and the Keychain, permissions (TCC), passkeys and Cowork are expected to work exactly as in the original. Profiles are told apart by a Finder custom icon: Claude's icon with a badge in the profile color. The icon is metadata on the bundle directory, outside the sealed contents.

The two existing modes stay as they are.

| Mode | Stored field | What runs |
|------|--------------|-----------|
| Script launcher (default) | both false | `/Applications/Claude.app` |
| Wrapper (re-signed) | `distinctDockIcon: true` | `/Applications/Claude (<Name>).app`, a copy re-signed ad hoc |
| **Signed clone (new)** | `signedClone: true` | `profiles/<id>/app.noindex/Claude (<Name>).app`, a copy with its signature untouched |

---

## 1. Premises and decisions

| # | Topic | Decision |
|---|-------|----------|
| P1 | The signed clone is added on top of what the app already does. | Profile storage, auth paths, usage stats and session moves are left as they are. |
| P2 | Data paths and identifiers are ai-profiles' own (`~/Library/Application Support/ai-profiles/`, `app.ai-profiles.*`). | Kept, so existing profiles never have to move. |
| P3 | A signed clone is available for Claude profiles on macOS only. | Gated by `AppSpec.supports_signed_clone` in Rust and `appSpecs[app].signedClone` in TypeScript. |
| P4 | The script and wrapper modes keep their code and behavior. | Kept. |
| Q2 | Whether to install the background watch by default. | Installed while any profile uses a signed clone. It acts on its own only on strays younger than 20 seconds. |
| Q3 | Whether to accept that `codesign --strict` fails. | Accepted. If the runtime checks (C1, C4, F1) show a problem, the mode is put on hold. |
| Q4 | Whether to ship if the Dock doesn't show the icon on macOS 26 (D2). | Ship. Keeping the signature and permissions is worth it on its own. |
| U1 | A Claude Desktop update must never require an update of this app. | §5.8 |

**Never touched**: `~/Library/Application Support/Claude`, `~/.claude`, `~/.claude.json`, `~/Library/Application Support/Claude-3p` (present on this Mac), Keychain items, the contents of each profile's `gui-data` and `cli-config`, and `/Applications/Claude.app` (only ever read).

---

## 2. What does not change

| Feature | Paths it uses | With a signed clone |
|---------|---------------|---------------------|
| Profile storage | `profiles.json`, `profiles/<id>/gui-data`, `profiles/<id>/cli-config` | Same. Only the `signedClone` field is added. |
| Desktop sign-in | `--user-data-dir=<gui-data>` and the Keychain item "Claude Safe Storage" | Same `gui-data`. The designated requirement is the same, so the Keychain ACL is expected to let it through (check C1). |
| CLI and Code tab sign-in | The Keychain item `Claude Code-credentials-<sha256(cli-config)[:8]>`, chosen by `CLAUDE_CONFIG_DIR=<cli-config>` | The path string comes from **the same function** as the CLI wrapper's, `paths::cli_config_dir(id)`, so the Keychain item is the same (check C2). |
| Usage stats | `usage/credentials.rs` reads `cli-config` | Unchanged |
| Session list, move, archive | `sessions/home.rs` uses `profiles::paths()` | Paths unchanged. The running-instance check now counts strays (§5.6). |
| Account display | `accounts.rs` | Unchanged |

---

## 3. What was measured

### 3.1 Static probe (nothing launched)

`/Applications/Claude.app` was cloned with `cp -Rc` into a `*.noindex` directory and given an icon with `NSWorkspace.setIcon`.

| Item | Result |
|------|--------|
| `cp -Rc` | 1.35 s. `/Applications` and `~/Library` are on the same volume (disk3s5). |
| Extended attributes copied | `com.apple.macl`, `com.apple.provenance` |
| Bundle root after the icon | An `Icon\r` file appears, and the bundle directory gets `com.apple.FinderInfo`. |
| CDHash, designated requirement (DR) | **The same** as the original |
| `codesign --verify`, `--verify --deep`, `-R=<original DR>` | **Pass** |
| `spctl --assess --type execute` | **Accepted** (Notarized Developer ID, Anthropic PBC Q6L2SF6YDW) |
| `codesign --verify --strict` | **Fails**: `Disallowed xattr com.apple.FinderInfo` |

### 3.2 E2E test after implementation (nothing launched)

`launchers::clone::tests::a_built_clone_keeps_the_vendor_signature_and_carries_its_icon` passes with `AI_PROFILES_E2E=1` (8.3 s). It has the real `clone::ensure` make a clone, give it its icon and pass the four-step verification. It then checks that a second `ensure` changes nothing (same inode), and that the clone is removed at the end.

In short: the signature itself is preserved, but to strict validation the bundle root carries extra metadata. Whether any runtime check (Keychain, TCC, Squirrel updates) objects to that can only be found out by running it.

---

## 4. The modes compared

| | Script | Wrapper | **Signed clone** |
|---|---|---|---|
| Code signature | Anthropic's | Ad hoc, team entitlements removed | Anthropic's, byte for byte; only `--strict` fails |
| Bundle ID | Anthropic's | `app.ai-profiles.claude.profile.<id>` | Anthropic's (shared with the original) |
| Dock icon while running | Stock orange | Profile badge | Finder custom icon with the profile badge; shown on macOS 26? (check D2) |
| Sign in again after switching mode | — | Yes | Expected not to (C1) |
| TCC, passkeys, Cowork | Work | Permissions asked again, passkeys may fail, no Cowork folders | Expected to work (C4–C6) |
| Claude's own updates | The original updates | Not possible; rebuilt when the original's version changes | Installed into the clone, which can get ahead of the original |
| When started by macOS (Dock, update) | The original starts without arguments (existing limitation) | The shim adds the arguments back | The clone starts without arguments — a stray, handled in §5.6 |

---

## 5. Design

### 5.1 Data model

- `Profile.signed_clone: bool`, stored as `signedClone`, false by default. `ProfilePatch.signed_clone: Option<bool>` was added too.
- `profiles::validate_desktop_mode` checks, on create and on `patched`:
  - the app's spec has `supports_signed_clone`, and this is macOS;
  - it is not on together with `distinct_dock_icon`.
- If a hand-edited `profiles.json` has both true, launching and launcher generation both prefer the signed clone.
- **Backward compatibility**:
  - Builds of the app that predate the field ignore `signedClone`. They rebuild such a profile as a script launcher and open it that way, on the same data paths.
  - When such a build saves `profiles.json`, the field is lost. So the two apps must not share the same data.
- In the frontend, `signedClone?` is optional. The forms send it only when it is on or being turned off, so a form for any other profile submits exactly what it always did.

### 5.2 Files on disk

```
~/Library/Application Support/ai-profiles/
  profiles.json                          # gains the signedClone field
  clone-watch.log                        # the clone watch's log
  profiles/<id>/gui-data/                # unchanged
  profiles/<id>/cli-config/              # unchanged
  profiles/<id>/app.noindex/             # new (a cache: deleted, it is made again on the next launch)
      Claude (<Name>).app                # the clone, with its Finder icon
      clone.json                         # {clonedFromVersion, color, builtBy}
      .lock                              # cross-process lock
/Applications/Claude (<Name>).app        # script launcher shape; the script hands off to ai-profiles
~/Library/LaunchAgents/app.ai-profiles.clone-watch.plist   # only while a profile uses a signed clone
```

- **Ownership by path.** The clone keeps Anthropic's identifier and its Info.plist is sealed. So the existing identifier check (`is_ours`) can't recognize it, and nothing can be recorded inside it. What is known about it goes in `clone.json`, outside the bundle.
- **`.noindex`** keeps Spotlight from indexing the folder, which closes one way of opening the clone without its arguments.
- **The file name `Claude (<Name>).app`** is expected to become the Dock and ⌘-Tab label (check D3). When the profile is renamed, the next `ensure` renames the clone rather than cloning again, so a version it updated itself to is not lost.

### 5.3 Launch flow

```
GUI "Open"   ──┐
Launcher in Spotlight ──> ai-profiles --open-profile <id> ──┐
Clone watch ────────────────────────────────────────────────┴─> launch::open_profile_with
  1. Settle strays (clone::settle_strays)
     - 20 s old or younger: quit with SIGTERM, then carry on
     - older: return AppError::StrayRunning
       · the GUI shows a "Quit and reopen" toast
       · the launcher shows an osascript confirmation dialog
  2. If running_pid(gui-data, executable) finds the profile: focus_pid (reopen Apple event + activate), done
  3. clone::ensure makes or refreshes the clone (policy in §5.4)
  4. open -n -a <clone> --args --user-data-dir=<gui-data>, with env CLAUDE_CONFIG_DIR=<cli-config>
  5. watch_start watches it come up (the same logic as a wrapper's)
  6. If 3–5 fail: return a Bypass and open the stock app instead, so the profile always opens
```

- The launch always goes **through LaunchServices (`open`)**. Executed directly, the clone would have ai-profiles as its responsible process for TCC, and permission prompts would name ai-profiles.
- **`-n` is required.** The clone and the original share a bundle ID, so without it `open` brings the running original forward.
- If the launcher (`/Applications/Claude (<Name>).app`) can't find ai-profiles at the recorded path, it opens the original the old script way.
- A handoff launcher left behind for a profile without a signed clone would loop forever. So `open_script_launcher` opens the stock app instead of such a launcher, and the launcher is rebuilt in the right shape on the next start (`outdated` detects a shape mismatch or a host path that has gone).

### 5.4 Clone refresh policy (`clone::plan`)

| State | Action |
|-------|--------|
| No clone | Build one |
| In use (main process, helpers or ShipIt) | **Leave it alone** |
| Clone older than the original | Build again (moves it forward) |
| Same version, icon missing or in another color | Build again. The icon goes on a fresh bundle that has never run, and APFS block sharing is regained |
| Clone newer than the original (it updated itself) | Keep it. Only if the icon is missing or in another color, try to set it in place; on failure, launch without the color |
| Versions can't be compared | Launch what is there |

**Building one:**

1. Take the cross-process file lock (`.lock`, `File::lock`).
2. `cp -Rc` into a hidden staging directory (no `.app` suffix). Check it is on the same volume as the original, and that the Info.plist and executable bytes match.
3. Draw the icon with `icons::render_icns` (the same badge as the launcher) and apply it with `NSWorkspace.setIcon`.
4. Verify: `codesign --verify --deep` (not `--strict`), the original's DR is satisfied, the CDHash matches, and `spctl` accepts it.
5. **Check again, right before the swap, that nothing has started using the old clone.**
6. Move the existing clone aside to `.replaced-*`, rename the staging directory into place, then delete the old one. If deleting is blocked, it is left and cleared by the next `ensure`.
7. Write `clone.json`.

If the bundle holds files not owned by the user (an update that ran as root), the replacement is refused. The `sudo` command to fix it is shown but never run.

### 5.5 Icon

- **It is a badge, not a recolor.** Claude's own icon stays as it is, and a circular badge in the profile color, with a white ring, sits at its bottom right. This is the app's existing launcher icon (`icons::render_icns`), so the launcher in Spotlight and the running Dock tile look the same. The rest of the icon is not recolored.
- Changing the color while the profile runs updates the launcher's icon at once, and the clone's icon on its next launch.
- Whether the macOS 26 Dock prefers the Finder custom icon over the asset catalog (`CFBundleIconName`) is not known yet (check D2).

### 5.6 Lost arguments (strays)

A stray is a process whose executable is `profiles/<id>/app.noindex/*.app/Contents/MacOS/<exec>` with no `--user-data-dir=` on its command line (`clone::stray_profile`). Ways one appears:

- S1: ShipIt relaunches the clone after it updated itself
- S2: a click on its Dock tile or a Recents entry
- S3: windows reopened at login
- S4: LaunchServices sends a `claude://` link to the clone
- S5: "Open at login" turned on inside the clone
- S6: S1 while the default instance already runs, so the stray hands itself to that instance and quits at once

| Handling | Where | What it does |
|----------|-------|--------------|
| At launch | `launch::open_signed_clone` | Strays 20 s old or younger are quit; older ones are asked about (§5.3). SIGKILL is never used. |
| When opening Default | `commands::open_default_gui` | Re-homes fresh strays first; otherwise the stock launch would hand itself over to the stray. |
| Session checks | `sessions::instance::desktop_pids` | A stray uses the stock data, so it **counts as the default instance**. |
| The clone watch | `clone_watch` (`ai-profiles --watch-clones`, every 2 s) | Re-homes fresh strays. Notifies once per pid about an older stray. For S6: when a clone's version changed and nothing runs from it 30 s later, sends a notification. |

- **Why S6 is not reopened automatically**: Squirrel also installs updates silently when the user simply quits. So from outside there is no telling whether a relaunch was meant. Instead the notification says "updated; if it didn't reopen, open it again", which is right either way. A clone this app made itself matches `clonedFromVersion` in `clone.json`, so it causes no notification.
- **Installing and removing the watch**: `clone_watch::sync` runs after a profile is created, edited or deleted, after a surface is switched, and when the app starts.
  - If any GUI profile uses a signed clone, the watch is installed with `launchctl bootstrap`.
  - If none does, it is booted out and its plist deleted.
  - If the executable has moved, the plist is rewritten.
  - The user can switch it off in System Settings → General → Login Items & Extensions.

### 5.7 Switching modes, and going back

| Switch | What happens | Data |
|--------|--------------|------|
| Script → signed clone | The clone is built **first**; if that fails the launcher is left as it was. Then the launcher is rebuilt in the handoff shape and the watch installed. | Untouched |
| Signed clone → script or wrapper | The launcher is rebuilt, and the clone (a cache) deleted unless in use; one in use is left and cleared later. The watch is removed with the last such profile. | Untouched |
| Wrapper ↔ signed clone | Refused while the wrapper runs, by the existing `ensure_wrapper_not_running`. | Untouched |
| Rename or recolor | Only the launcher is rebuilt; the clone changes on its next launch. | Untouched |
| Desktop surface off | The launcher is removed, and the clone too unless in use. | Untouched |
| Profile deleted | The existing policy (not refused while running). The clone goes with the profile folder, to the Trash or deleted. | As the user chose |

**Never done**: moving or deleting account data automatically; replacing, changing or moving a clone that is running; `sudo`; writing to `/Applications/Claude.app`.

### 5.8 Independent of Claude Desktop updates

A Claude Desktop update never requires rebuilding or updating claude-switch. Nothing about Claude is fixed in the code; everything is read from the installed app when it is needed.

| Value | Read from |
|-------|-----------|
| Version | `CFBundleVersion` (`wrapper::bundle_version`), compared on every launch |
| Executable name | `CFBundleExecutable` (`clone::vendor_executable`), used for cloning, stray detection and finding the running instance |
| Signing requirement | The original's DR via `codesign -d -r-`; no team ID is hardcoded |
| CDHash | The original's and the clone's, compared each time |
| Icon | Drawn again from the original's icon whenever a clone is made |

- **When the original updates**: profiles that aren't running are cloned forward on their next launch. A running profile is left alone and updates itself. Both follow from the version comparison.
- **It still opens if something breaks**: should a new Claude make the clone unusable (a signature change, a restriction on where it may run), the launch falls back with a `Bypass` to the stock app, and the profile still opens.
- **Remaining assumptions**, which the whole app relies on — if one breaks, the existing modes need fixing too:
  - The app is at `/Applications/Claude.app` (`AppSpec.gui_bundle_candidates`).
  - Electron honors `--user-data-dir`.
  - The Code tab follows `CLAUDE_CONFIG_DIR`.
- This app's own updater now checks this repository's Releases. There are none, so it never updates itself.

### 5.9 Known limitations

- The bundle ID is shared, so a `claude://` sign-in callback goes to whichever instance LaunchServices picks. For a profile's **first sign-in, quit every other Claude** (the existing modes have the same limitation).
- Notification settings, default-app choices and login items are shared with the original.
- The original and every clone share the ShipIt cache (`~/Library/Caches/com.anthropic.claudefordesktop.ShipIt`) (check F4).
- A clone that updated itself becomes a separate copy of about 650 MB. It is not excluded from Time Machine.
- With a development build (`tauri dev`), the watch points at the development binary. Running the installed app corrects it.

---

## 6. Code map

| File | Role |
|------|------|
| `src-tauri/src/launchers/clone.rs` | Refresh plan (`plan`), building and verifying (`ensure`, `build`, `verify`), the icon, ownership, stray detection and quitting |
| `src-tauri/src/launchers/clone_watch.rs` | LaunchAgent plist, `sync`, the `--watch-clones` loop, notifications |
| `src-tauri/src/launch.rs` | `open_profile_with`, `open_signed_clone`, `CloneEffects` and `launch_clone_with` (fallback), `open_instance_of` |
| `src-tauri/src/launchers/script.rs` | `handoff_launcher_script`, `hands_off`, `handoff_host` |
| `src-tauri/src/launchers/gui.rs` | The `generate` branch, `outdated`'s handoff-shape and host-path checks, `hands_off` |
| `src-tauri/src/profiles.rs` | The `signed_clone` field, `validate_desktop_mode`, removing the clone when the desktop surface goes off |
| `src-tauri/src/paths.rs` | `profiles_root`, `clone_dir`, `clone_bundle_path`, `CLONE_DIR_NAME` |
| `src-tauri/src/sessions/instance.rs` | Counting strays as the stock instance |
| `src-tauri/src/commands.rs`, `cli.rs`, `main.rs`, `lib.rs`, `error.rs` | Command arguments (`signedClone`, `quitStray`), `--watch-clones`, `sync` at startup, `StrayRunning` |
| `src/features/profiles/components/*` | The option row, `SignedCloneDialog`, `useSignedCloneOption`, dialog wiring |
| `src/features/profiles/api/use-profile-last-used.ts` | The "Quit and reopen" toast on `StrayRunning` |

---

## 7. Tests

- **Rust**: 727 pass, 30 of them new. They cover:
  - the refresh plan table, version comparison, telling strays from helpers, parsing `ps` elapsed times, parsing the DR and CDHash;
  - reading and writing the record, clearing leftovers, adopting a renamed clone, leaving a clone in use alone;
  - generating, recognizing and parsing the handoff script's host path (including quoted paths), and the `outdated` shape checks;
  - the four fallback cases (`FakeClone`), counting strays as the stock instance, the watch's plist, when it is wanted, version-change detection, and parsing `--watch-clones`.
- **E2E** (`AI_PROFILES_E2E=1`): `a_built_clone_keeps_the_vendor_signature_and_carries_its_icon` checks a real clone, its verification and its removal. The app is never launched.
- **Frontend**: 689 pass, 9 of them new: the edit plan, the two options clearing each other in the create and edit dialogs, the option being hidden for ChatGPT, and the explanation dialog.
- **Static checks**: `cargo clippy -- -D warnings` (as CI runs it), `cargo fmt --check`, `tsc`, `biome check` and `fallow` all pass.
  - `cargo clippy --all-targets` reports two errors (`test_support.rs`, `profiles.rs:788`); both are in existing test code.

Runtime checks (sign-in, permissions, the Dock, updates) follow [signed-clone-verification.md](signed-clone-verification.md).

---

## 8. Changes from the first draft

- No `DesktopMode` enum: one bool plus `validate_desktop_mode` was enough.
- A JSON file with both flags true is not corrected on save; launching and building prefer the signed clone instead.
- S6 is reported in a notification rather than reopened automatically (why: §5.6), so nothing is ever reopened against the user's intent.
- Deleting a profile follows the existing policy (not refused while running) rather than being refused.
- A file lock and a re-check right before the swap guard against concurrent `ensure` calls for the same profile (GUI, launcher, watch).
- The app's updater points at this repository, and release workflows this repository can't run are removed.
