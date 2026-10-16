# Releasing

Releases are built on a Mac and published by hand with [`scripts/release.sh`](../scripts/release.sh). It builds the universal app, notarizes it, makes the files the built-in updater reads, and with `--publish` uploads them to a GitHub release.

## How the updater finds a release

The app reads `https://github.com/rokrokss/claude-switch/releases/latest/download/latest.json` at launch, every 6 hours, and from Settings → System. When that file names a newer version, the app offers to restart and install `claude-switch.app.tar.gz` from the same release. Before installing, it checks the tarball's signature against the public key in `apps/claude-switch/src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). It refuses a tarball signed with any other key.

Each release therefore carries three files:

| File | Used by |
|------|---------|
| `claude-switch_<version>_universal.dmg` | People installing by hand |
| `claude-switch.app.tar.gz` | The updater. The notarized, stapled app, signed with the updater key |
| `latest.json` | The updater. Version, notes, date, and the tarball's URL and signature for `darwin-aarch64` and `darwin-x86_64` |

## One-time setup

- **Developer ID certificate**: `Developer ID Application: Hyungrok Kim (87G5R48C73)` in the login keychain (`security find-identity -v -p codesigning`).
- **Notarization credentials**: a notarytool keychain profile named `claude-switch`:

  ```sh
  xcrun notarytool store-credentials claude-switch --apple-id <apple-id> --team-id 87G5R48C73
  ```

- **Updater signing key**: the private key is `~/.tauri/claude-switch.key`. Its password is in the login keychain under the service `claude-switch-updater`. Its public half, `~/.tauri/claude-switch.key.pub`, is the `pubkey` in `tauri.conf.json`. **Back up the key and the password**, for example in a password manager. Without them no new release can update existing installs. A new key would mean every user downloads one release by hand, as with 1.4.0 below.
- **GitHub CLI** signed in with write access to the repository (`gh auth status`), and `jq`.
- **iCloud provisioning profile**: the Developer ID profile `Claude Switch Developer ID`, for the App ID `app.claude-switch` with the iCloud container `iCloud.app.claude-switch`, kept at `~/.tauri/claude-switch.provisionprofile`. Set `ICLOUD_PROVISIONING_PROFILE` to its path when running the script. [`scripts/icloud-config.sh`](../scripts/icloud-config.sh) checks it, signs the app with the entitlements it allows, and embeds it. Without the variable, the release builds without iCloud and its Settings say sync isn't available. Developer ID profiles don't expire for years, but changing the App ID's capabilities invalidates the profile; download it again after any change.

The script's defaults can be overridden with `SIGNING_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `NOTARY_PROFILE` and `APPLE_SIGNING_IDENTITY`.

## Each release

1. Bump the version in `apps/claude-switch/src-tauri/tauri.conf.json`, `apps/claude-switch/package.json` and `apps/claude-switch/src-tauri/Cargo.toml`, run `cargo check` in `src-tauri` so `Cargo.lock` follows, and add the release to `apps/claude-switch/CHANGELOG.md`. Commit and push.
2. Write the release notes to a file. They become the GitHub release's text and the `notes` in `latest.json`.
3. Run the script without `--publish`:

   ```sh
   ICLOUD_PROVISIONING_PROFILE=~/.tauri/claude-switch.provisionprofile scripts/release.sh notes.md
   ```

   It builds, notarizes, staples, and writes the three files to `apps/claude-switch/src-tauri/target/release-assets/v<version>/`. It then prints the DMG's SHA-256 and the `gh` commands that would publish them.
4. Check the files:

   ```sh
   cd apps/claude-switch/src-tauri/target
   xcrun stapler validate universal-apple-darwin/release/bundle/macos/claude-switch.app
   xcrun stapler validate release-assets/v<version>/claude-switch_<version>_universal.dmg
   tar tzf release-assets/v<version>/claude-switch.app.tar.gz | grep -c 'claude-switch.app/Contents/CodeResources'   # 1: the stapled ticket is in the tarball
   jq . release-assets/v<version>/latest.json
   ```

5. Publish: run the two printed `gh` commands. The script creates the release as a draft, uploads the files, then publishes it, so `releases/latest` only moves once `latest.json` is there. (`scripts/release.sh notes.md --publish` does the same in one go, but builds and notarizes again.)

### Why the script repacks the app

The bundler makes `claude-switch.app.tar.gz` while it builds, before the app is notarized. The script notarizes afterwards, with the notarytool keychain profile. Stapling then writes the ticket into the app (`Contents/CodeResources`), and the script packs and signs the tarball again from the stapled app.

## Updating from 1.4.0

1.4.0 shipped before the project had its own updater key. It carries the upstream project's public key, and its release has no `latest.json`. It reads the same `latest.json` URL, so it will announce the next release, but it can't verify the signature. Installing from the app fails, and its users have to download that release's DMG by hand once. Say so in the notes of the first release that has `latest.json`, for example:

> **Updating from 1.4.0:** 1.4.0 can't install updates itself. It shows this update, but installing it from the app fails. Download the `.dmg` below and drag claude-switch to Applications once; from this release on, claude-switch updates itself.
