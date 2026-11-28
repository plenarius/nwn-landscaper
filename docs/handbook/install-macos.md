# Install on macOS

The Landscaper for macOS is one app for Apple silicon and Intel Macs, on macOS 11 and newer.

## Install

1. Download `nwn-landscaper-0.7.0-macos-universal.dmg` from the
   [releases](https://github.com/plenarius/nwn-landscaper/releases).
2. Open it and drag **NWN Landscaper** onto **Applications**.
3. Eject the disk image.

## Start it

Start **NWN Landscaper** from Applications or Launchpad. The first time, macOS asks whether to open an app downloaded from the
internet: click **Open**. A window opens on *Starting the Landscaper…*, then shows the editor. The first start takes a minute, and
asks for the game's folder if it can't find it: [The first start](first-start.md).

**macOS also asks once whether NWN Landscaper may access your Documents folder.** The game's user folder,
`Documents/Neverwinter Nights`, is there: click **Allow** (or **OK**). If you said no, turn it on in **System Settings > Privacy &
Security > Files and Folders > NWN Landscaper**.

Only one copy runs: starting it again brings its window forward. Quit it (Cmd+Q) or close its window to stop it, engine and all.

## Where things are

- **Your areas, workspaces and texture library:** the data folder, `~/.local/share/landscaper`, the same folder the program in
  the archive uses. **Help > Open the data folder** opens it.
- **Exports:** your game user folder's `hak` and `modules` folders.
- **Package for your module…** saves its zip in your Downloads folder.

## Update or remove it

- **Update:** the app does it. When a new release is out, a dot shows beside the version next to the title, and its menu
  offers **Update to X.Y.Z** with what is new. Click it and say yes: the app downloads the new app, checks its signature,
  replaces itself and starts again. The data folder stays as it is. An app from 0.7.3 or before doesn't look for updates: drag
  the new release's app onto Applications once, replacing the old one, and it updates itself from then on.
- **Remove:** drag **NWN Landscaper** from Applications to the Trash. The data folder stays; delete `~/.local/share/landscaper`
  yourself to remove your areas too.
