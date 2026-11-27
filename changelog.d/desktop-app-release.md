- **The desktop app's installers in each release** (step 4 of `docs/DESIGN-desktop-app.md`): an NSIS installer for Windows
  (for one user, no administrator), a `.dmg` with one universal app for macOS 11 and newer, and an AppImage and a `.deb` for
  Linux, built by Tauri with the editor of the same tag inside. On a version tag Tauri signs them with the identities the program
  already uses (Azure Artifact Signing on Windows; a Developer ID on macOS, notarized, with Apple's ticket stapled to the app and
  the `.dmg`). The release checks each installer by installing it and starting the app (`scripts/app-smoke.sh`): its window must
  load the editor at `/?app` from the engine inside it, a second start must only bring it forward, and the engine must stop with
  the app. The archives with the program alone stay beside the installers. The app's log names each page its window loads.
