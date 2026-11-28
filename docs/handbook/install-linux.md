# Install on Linux

Two downloads for 64-bit Linux, both with the same app: an AppImage that runs on most desktops, and a `.deb` for Debian 12, Ubuntu
22.04 and newer. Both need glibc 2.36 or newer.

## The AppImage

1. Download `nwn-landscaper-0.7.0-linux-x86_64.AppImage` from the
   [releases](https://github.com/plenarius/nwn-landscaper/releases).
2. Make it executable, and start it:

    ```sh
    chmod +x nwn-landscaper-0.7.0-linux-x86_64.AppImage
    ./nwn-landscaper-0.7.0-linux-x86_64.AppImage
    ```

    Or, in your file manager, tick *Allow executing file as program* in the file's properties and double-click it.

If it says it can't mount itself (no FUSE on the system), start it with `APPIMAGE_EXTRACT_AND_RUN=1` in front: it unpacks itself
and runs from there.

The AppImage updates itself. When a new release is out, a dot shows beside the version next to the title, and its menu offers
**Update to X.Y.Z** with what is new. Click it and say yes: the app downloads the new AppImage, checks its signature, puts it in
place of the old file (same name, same folder, so keep it in a folder you can write to) and starts again. An AppImage of 0.7.3
or before doesn't look for updates: download the new one yourself once.

## The .deb

```sh
sudo apt install ./nwn-landscaper_0.7.0_amd64.deb
```

Start **NWN Landscaper** from your applications menu, or `nwn-landscaper` from a terminal. To remove it,
`sudo apt remove nwn-landscaper`.

**To update, install the new release's `.deb` the same way.** The app can't do it for you, as installing a `.deb` needs an
administrator, but it tells you: when a new release is out, a dot shows beside the version next to the title, and its menu offers
**Download X.Y.Z's .deb**, which opens the release's page.

## Start it

A window opens on *Starting the Landscaper…*, then shows the editor. The first start takes a minute, and asks for the game's
folder if it can't find it: [The first start](first-start.md). Only one copy runs: starting it again brings its window forward.
Close the window to stop it, engine and all.

Check the downloads against `SHA256SUMS` on the release page (`sha256sum -c SHA256SUMS
--ignore-missing` in the download folder).

## If the window stays blank

The window draws with WebKitGTK, the system's web engine. With NVIDIA's own driver it can show a blank or flickering window; the
app turns off the part of WebKitGTK that causes it when it finds that driver. On another system with the same trouble, start the
app with `LANDSCAPER_WEBVIEW_SAFE=1` in front, which turns off more of it. If the 3D view still doesn't draw, use
[the program alone](program-alone.md) with your own browser.

## Where things are

- **Your areas, workspaces and texture library:** the data folder, `~/.local/share/landscaper` (or `landscaper` in
  `$XDG_DATA_HOME`), the same folder the program in the archive uses. **Help > Open the data folder** opens it.
- **Exports:** your game user folder's `hak` and `modules` folders.
- **Package for your module…** saves its zip in your Downloads folder.
