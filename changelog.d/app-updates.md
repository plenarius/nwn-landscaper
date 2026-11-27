<!-- markdownlint-disable-file MD041 -->
- **The app updates itself.** At start it looks for a newer release; when there is one, the editor's version menu offers
  **Update to X.Y.Z** with what is new, and on a yes the app downloads it, checks its signature against the key built into the
  app, installs it and starts again (the installer on Windows, the app on macOS, the AppImage on Linux). An app installed from
  the `.deb` is told to install the new `.deb`. Each release now carries `latest.json`, the update files' `.sig` signatures and
  the macOS app as a `.app.tar.gz`. See the install pages: [Windows](../docs/handbook/install-windows.md#update-or-remove-it),
  [macOS](../docs/handbook/install-macos.md#update-or-remove-it), [Linux](../docs/handbook/install-linux.md).
