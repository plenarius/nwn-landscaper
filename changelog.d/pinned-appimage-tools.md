- **The AppImage builds from pinned tools.** Tauri's AppImage tools and the AppImage runtime that appimagetool puts at the start
  of the file come at fixed versions with checked checksums (`deploy/release/appimage-tools.sh`); before, one tool and the
  runtime came from releases that move with every upstream change. The release checks that Tauri fetched nothing of its own
  and that the AppImage starts with the pinned runtime.
