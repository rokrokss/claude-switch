# Verifying the signed clone mode

The signed clone mode was **implemented on predictions** on 2026-10-14. So far only static checks have been done: cloning, signatures, Gatekeeper, and the unit and E2E tests. This guide lists the runtime checks that remain, in an order you can follow later. The design behind them is in [signed-clone-design.md](signed-clone-design.md).

**Ground rules**

- Use a **test profile and a test account**. Sign in yourself.
- This app never writes to `~/Library/Application Support/Claude`, `~/.claude`, `Claude-3p` or the Keychain. In the stray scenarios (section G), though, **Claude itself** briefly starts on the default data. Nothing is deleted or moved, but your default account may show for a moment.
- Each "If it fails" says what to change or report. Record results in the table at the end.

---

## 0. Setup

### 0.1 Build and install

```bash
pnpm install
```

```bash
pnpm --filter claude-switch tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

The result is `apps/claude-switch/src-tauri/target/release/bundle/macos/claude-switch.app`. Copy it to `/Applications`, and open it the first time with right-click → Open (it is a local build without notarization).

- The clone watch and the launchers record **the path of the claude-switch executable that is running**, so verify with the app installed in `/Applications`. `tauri dev` works too, but the paths then point at the development build.
- If another build of the app is installed, **don't run both** on the same data. A build that doesn't know `signedClone` drops it when it saves.

### 0.2 Test profile and shell variables

1. In claude-switch, create a profile: for example name `CloneTest`, app Claude, desktop on, CLI on, Signed clone **off** (so that C1 can be done first).
2. Find its id:

```bash
plutil -p "$HOME/Library/Application Support/claude-switch/profiles.json" | grep -E '"(id|name|signedClone|distinctDockIcon)"'
```

```bash
export ID=<the id found above>
```

```bash
export P="$HOME/Library/Application Support/claude-switch/profiles/$ID"
```

### 0.3 Commands used throughout

The profile's processes (clone, original, strays):

```bash
ps -ax -o pid=,etime=,command= | grep -E 'app.noindex|Claude.app/Contents/MacOS/Claude' | grep -v -E 'Helper|grep'
```

The clone, its record, and its version:

```bash
ls -la "$P/app.noindex" && cat "$P/app.noindex/clone.json" && /usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$P/app.noindex/"*.app/Contents/Info.plist
```

The clone watch's state:

```bash
launchctl print "gui/$(id -u)/app.claude-switch.clone-watch" | head -20
```

The clone watch's log:

```bash
tail -f "$HOME/Library/Application Support/claude-switch/clone-watch.log"
```

The clone's signature (three of the four checks, by hand):

```bash
codesign --verify --deep --verbose=1 "$P/app.noindex/"*.app && spctl --assess --type execute -vv "$P/app.noindex/"*.app && codesign -dvvv "$P/app.noindex/"*.app 2>&1 | grep CDHash && codesign -dvvv /Applications/Claude.app 2>&1 | grep CDHash
```

The automated check that clones the real Claude.app, verifies it and deletes it, without launching it:

```bash
cd apps/claude-switch/src-tauri && CLAUDE_SWITCH_E2E=1 cargo test a_built_clone_keeps_the_vendor_signature -- --nocapture
```

---

## 1. First: the checks that decide the mode (★)

These five decide whether the mode is worth having. About 30 minutes.

| Order | ID | In one line |
|-------|----|-------------|
| 1 | C1 | Switching from script to signed clone needs no new sign-in |
| 2 | B2 | The Code tab uses the profile's `cli-config` |
| 3 | B4, C4 | Permission prompts name "Claude", and permissions granted to the original carry over |
| 4 | D2 | The Dock shows the profile's badged icon |
| 5 | F1, F3 | Done when the next Claude update comes out |

---

## 2. Full checklist

### A. Signature, Gatekeeper, LaunchServices

**A1 — Signature preserved** (passed in the static checks)
- Steps: run "The clone's signature" from 0.3.
- Expected: `valid on disk`, `accepted / Notarized Developer ID`, and the same CDHash twice. `codesign --strict` failing is **expected** (FinderInfo).

**A2 — No Gatekeeper dialog on first launch**
- Steps: open the profile right after its clone was made.
- Expected: no dialog. Note how long it takes to come up (the first-run assessment can add a few seconds).
- If it fails: record `xattr -l "$P/app.noindex/"*.app`. If there is a quarantine attribute, add a step that removes it after cloning.

**A3 — The default handler stays with the original**
- Steps: after opening the profile once, run:

  ```bash
  swift -e 'import AppKit; print(NSWorkspace.shared.urlForApplication(toOpen: URL(string: "claude://x")!)?.path ?? "none"); print(NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.anthropic.claudefordesktop")?.path ?? "none")'
  ```

- Expected: both lines print `/Applications/Claude.app`.
- Check again **after the clone has become newer than the original** (after F1).
- If it fails: a clone path means the S4 risk is real. Consider unregistering the clone with `lsregister -u` when it quits.

**A4 — Not in Spotlight**
- Steps: run:

  ```bash
  mdfind 'kMDItemCFBundleIdentifier == "com.anthropic.claudefordesktop"'
  ```

- Expected: no `app.noindex` path in the results.

### B. Launching and isolation

**B1 — Finding the running profile**
- Steps: open the profile and run the process command from 0.3.
- Expected: one line, `…/app.noindex/Claude (CloneTest).app/Contents/MacOS/Claude --user-data-dir=…/gui-data`, with nothing after it.

**B2 — The environment reaches the app (★)**
- Steps:
  1. Before launching, note `stat -f %Sm ~/.claude/projects`.
  2. Start a session in the clone's Code tab, in any folder.
  3. Check `ls "$P/cli-config/projects"`.
- Expected: an entry appears in the profile's `cli-config/projects`, and the mtime of `~/.claude/projects` is unchanged.
- If it fails: `open` isn't passing the environment on to the app. Change `launch::open_instance_of` to use `NSWorkspaceOpenConfiguration` (arguments and environment).

**B3 — Why `-n` is needed** (optional)
- Steps: with the original Claude running, run `open -a "$P/app.noindex/"*.app` (without `-n`).
- Expected: the original's window comes forward, which is why `-n` is needed. If a new process does appear, that stray is handled as in section G.

**B4 — Who asks for permissions (★)**
- Steps: use the microphone (dictation) or attach a folder in Cowork, in the clone. Do it after each of the three ways in: the GUI, the launcher in Spotlight, and a relaunch by the clone watch.
- Expected: the prompt names **"Claude"**, not claude-switch or the `Claude (CloneTest)` launcher.
- If it fails: one of the paths launches outside LaunchServices. Record which.

**B5 — Running side by side**
- Steps: run Default, CloneTest (signed clone) and another script profile at once.
- Expected: each profile's Open button brings only its own window forward.

### C. Sign-in and permissions preserved

**C1 — No new sign-in after switching modes (★)**
- Steps:
  1. In script mode, sign in with Google using the test account, leaving the default Claude running. It should sign in through a system sheet, not a browser tab, while the clone watch's log says `a profile is signing in`, then `no profile is signing in` once it has (design §5.10).
  2. Quit the profile.
  3. In Edit profile, turn on Signed clone and save.
  4. Open the profile.
- Expected: still signed in, and no Keychain prompt for "Claude Safe Storage".
- If it fails: the strict validation failure matters at runtime. Put the mode on hold (design Q3).

**C2 — The CLI and the Code tab use the same Keychain item**
- Steps: run these two commands (no secret is printed):

  ```bash
  H=$(printf %s "$P/cli-config" | shasum -a 256 | cut -c1-8); echo "$H"
  ```

  ```bash
  security find-generic-password -s "Claude Code-credentials-$H" >/dev/null && echo present
  ```

- Expected: after signing in with `claude-<slug>`, it prints `present`, and the clone's Code tab is signed in too.

**C3 — Existing features**
- Steps: look at the profile's detail page.
- Expected: the usage meters, the session list and the account email show as before.

**C4 — TCC permissions carry over (★)**
- Steps: use, in the clone, a permission already granted to the original Claude (microphone, files and folders).
- Expected: not asked again.

**C5 — Passkey and hardware key sign-in**
- Expected: works (`keychain-access-groups` is preserved).

**C6 — Attaching folders in Cowork**
- Expected: works. This is what the wrapper mode can't do.

### D. Icons and names

**D1 — Launcher icon**
- Expected: `Claude (CloneTest)` shows Claude's icon with the profile badge in Spotlight and the Finder.

**D2 — Dock icon while running (★)**
- Steps: open the profile. Switch System Settings → Appearance → Icon & widget style through its options.
- Expected: the Dock tile shows Claude's icon with the profile's color badge. If it shows the plain original icon, record a failure (design Q4: the mode stays).

**D3 — Names shown**
- Expected: record what the Dock label, ⌘-Tab and the menu bar show (`Claude (CloneTest)` or `Claude`).

**D4 — Changing the color while running**
- Steps: change the profile's color while it runs. Quit it and open it again.
- Expected: the clone's mtime doesn't change while it runs (`stat -f %m "$P/app.noindex/"*.app`), and the new color shows after reopening.

### E. Focusing a running profile

**E1** — Opening a running profile again from the launcher or the GUI brings its window forward without a new process (one process, per the 0.3 command).

**E2** — After closing all its windows with ⌘W, opening it again brings a window back.

**E3** — No other profile's window, nor Default's, is brought forward.

### F. Updates (when a Claude update comes out)

**F1 — The clone updates itself (★)**
- Steps: keep CloneTest running until Claude offers an update, then restart to update. Keep the log open.
- Expected: the update completes, with no App Management prompt, and the clone's version goes up.

**F2 — The update's relaunch is put right**
- Expected: the relaunched stray is quit within 20 s and the profile opens again (`reopening it` in the log). The icon comes back: set in place if the clone is now newer than the original, from a fresh clone if they are the same.

**F3 — Updating while Default runs (★)**
- Steps: do F1 with Default Claude running.
- Expected: record whether the stray hands itself to Default and quits at once, and whether the "Claude updated itself in this profile…" notification arrives about 30 s later.

**F4 — The shared ShipIt cache**
- Steps: when the original and a clone take an update at the same time, record which target path the state file under `~/Library/Caches/com.anthropic.claudefordesktop.ShipIt/` names.

**F5 — Only ever forward after the original updates**
- Steps: after `/Applications/Claude.app` updates, open the profile.
- Expected: `clonedFromVersion` in `clone.json` and the clone's version are the new version. A clone already newer than the original is not cloned again (no downgrade).

**F6 — Files owned by root**
- Steps: after an update, run `find "$P/app.noindex" ! -user "$USER" | head`.
- Expected: if anything is listed, opening the profile shows the "belong to another user…" error with the command to fix it.

**F7 — App Management**
- Expected: record whether setting the icon again, moving the previous clone aside, or deleting it, on a clone that has run, is blocked or prompts. A previous clone that couldn't be deleted should be cleared on the next open (`.replaced-*` in `ls -la "$P/app.noindex"`).

### G. Relaunches from the Dock and at login

**G1 — A Dock tile kept in the Dock**
- Steps: right-click the running clone's Dock tile → Options → Keep in Dock. Quit the profile, then click the tile.
- Expected: it starts on the default data for a moment, is quit within 20 s, and reopens as CloneTest.

**G2 — Windows reopened at login**
- Steps: with the profile open and "Reopen windows when logging back in" on, log out and back in.
- Expected: the clone watch puts it right.

**G3 — `claude://` links**
- Steps: with no Claude running, run `open "claude://"`.
- Expected: record which bundle starts (S4).

**G4 — An older stray is asked about**
- Steps:
  1. Switch the watch off for a moment:

     ```bash
     launchctl bootout "gui/$(id -u)/app.claude-switch.clone-watch"
     ```

  2. Click the tile as in G1 and wait more than 30 s.
  3. Switch the watch back on:

     ```bash
     launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/app.claude-switch.clone-watch.plist"
     ```

- Expected: it isn't quit; only the "Running without its profile…" notification arrives. Pressing Open in claude-switch shows a "Quit and reopen" toast that puts it right. Opening from the launcher in Spotlight shows a confirmation dialog.

**G5 — The background item**
- Steps: when the watch is installed the first time, look for macOS's "Background Items Added" notification. Switch it off in System Settings → General → Login Items & Extensions, then run the state command from 0.3.
- Expected: it shows as switched off.

### H. Existing features still work

**H1** — Moving, archiving and restoring sessions work for a signed clone profile. Confirming "quit the instance" quits the clone.

**H2** — With a stray running, a session move to or from Default treats the stray as the default instance and offers to quit it.

**H3** — Creating, opening and editing script and wrapper profiles work as before.

**H4** — Data paths unchanged:
- Steps: before and after each mode switch, with nothing running, run the command below and compare.

  ```bash
  find "$P/gui-data" "$P/cli-config" -maxdepth 1 -exec stat -f '%i %N' {} + | sort
  ```

  Switch modes in this order: script → signed clone → script → wrapper → signed clone.
- Expected: the same files and inodes every time.

**H5** — Record whether switching between wrapper and signed clone needs a new sign-in.

**H6** — A running signed clone profile can be deleted (the existing policy). After the delete, the watch doesn't start that profile again.

### I. Safety boundaries

**I1** — During verification, keep this running in another terminal:

```bash
sudo fs_usage -w -f filesys | grep -E 'claude-switch|launcher' | grep -E 'Application Support/Claude[/ ]|Claude-3p|/\.claude'
```

- Expected: no write by our processes to those paths.

**I2** — With the clone running, changing the color, renaming, or switching modes leaves the clone bundle unchanged (check mtime and inode).

---

## 3. Undoing it if something goes wrong

- **Turn the mode off**: turn Signed clone off in Edit profile. The profile goes back to the script launcher. The clone is deleted unless it is running, and the data stays.
- **Remove the watch by hand**:

  ```bash
  launchctl bootout "gui/$(id -u)/app.claude-switch.clone-watch"; rm -f "$HOME/Library/LaunchAgents/app.claude-switch.clone-watch.plist"
  ```

- **Delete the clone by hand**: quit the profile first, then run the command below. The clone is a cache and is made again on the next launch.

  ```bash
  rm -rf "$P/app.noindex"
  ```

---

## 4. Results

| ID | Result (pass / fail / pending) | Date | macOS / Claude version | Notes |
|----|-------------------------------|------|------------------------|-------|
| A1 | Pass (static) | 2026-10-14 | 26.6.2 / 2.31226.1 | Only `--strict` fails, as expected |
| A2 | | | | |
| A3 | | | | |
| A4 | | | | |
| B1 | | | | |
| B2 ★ | | | | |
| B4 ★ | | | | |
| B5 | | | | |
| C1 ★ | | | | |
| C2 | | | | |
| C3 | | | | |
| C4 ★ | | | | |
| C5 | | | | |
| C6 | | | | |
| D2 ★ | | | | |
| D3 | | | | |
| D4 | | | | |
| E1–E3 | | | | |
| F1 ★ | | | | |
| F2 | | | | |
| F3 ★ | | | | |
| F4 | | | | |
| F5 | | | | |
| F6 | | | | |
| F7 | | | | |
| G1–G5 | | | | |
| H1–H6 | | | | |
| I1–I2 | | | | |
