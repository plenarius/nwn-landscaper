- **The engine is also a library** (`src/lib.rs`), for the coming desktop app (`docs/DESIGN-desktop-app.md`, step 1): the
  program is its command line, and `serve_local` serves the editor on this machine as `landscaper serve` does. The command line
  and `serve` work as before.
- **A settings file.** Every setting can also be in `settings.json` in the data folder, for a program started without a shell's
  environment (the Dock, a desktop menu); the environment wins over it. `docs/CONFIGURATION.md` says how ("The settings file").
