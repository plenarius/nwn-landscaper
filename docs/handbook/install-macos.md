# Install on macOS

The Landscaper for macOS is one app for Apple silicon and Intel Macs, on macOS 11 and newer.

## Install

With [Homebrew](https://brew.sh/), in Terminal:

```sh
brew install --cask plenarius/tap/nwn-landscaper
```

Or by hand:

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

- **Update:** drag the new release's app onto Applications and replace the old one, or with Homebrew,
  `brew upgrade --cask nwn-landscaper`. The data folder stays as it is.
- **Remove:** drag **NWN Landscaper** from Applications to the Trash, or `brew uninstall --cask nwn-landscaper`. The data folder
  stays; delete `~/.local/share/landscaper` yourself to remove your areas too (`brew uninstall --zap --cask nwn-landscaper`
  does both).
