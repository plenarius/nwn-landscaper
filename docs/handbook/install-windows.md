# Install on Windows

The Windows installer puts the Landscaper in for you alone: no administrator, nothing for other users of the computer.

## Install

1. Download `nwn-landscaper-0.7.0-windows-x86_64-setup.exe` from the
   [releases](https://github.com/plenarius/nwn-landscaper/releases).
2. Run it. If SmartScreen says it protected your PC, click **More info**, then **Run anyway**.
3. It installs into `%LOCALAPPDATA%\NWN Landscaper` and adds **NWN Landscaper** to the Start menu.

The window draws with WebView2, Microsoft Edge's engine, which Windows 10 and 11 have. When it is missing, the installer downloads
it.

## Start it

Start **NWN Landscaper** from the Start menu. A window opens on *Starting the Landscaper…*, then shows the editor. The first
start takes a minute, and asks for the game's folder if it can't find it: [The first start](first-start.md).

Only one copy runs: starting it again brings its window forward. Close the window to stop it, engine and all.

## Where things are

- **Your areas, workspaces and texture library:** the data folder, `%LOCALAPPDATA%\landscaper`. **Help > Open the data folder**
  opens it.
- **Exports:** your game user folder's `hak` and `modules` folders, as before (`Documents\Neverwinter Nights`).
- **Package for your module…** saves its zip in your Downloads folder.

A Landscaper you ran from the zip archive before uses the same data folder, so the app opens your areas as they were.

## Update or remove it

- **Update:** the app does it. When a new release is out, a dot shows beside the version next to the title, and its menu
  offers **Update to X.Y.Z** with what is new. Click it and say yes: the app downloads the new installer, checks its signature,
  installs it and starts again. Your areas and the data folder stay as they are. An app from 0.7.3 or before doesn't look for
  updates: run the new release's installer yourself once, and it updates itself from then on.
- **Remove:** uninstall **NWN Landscaper** in Windows' **Settings > Apps**, as any other program. The data folder stays; delete
  `%LOCALAPPDATA%\landscaper` yourself to remove your areas too.
