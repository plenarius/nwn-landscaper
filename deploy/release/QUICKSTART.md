# NWN Landscaper: quick start

This archive is the program alone: the engine, with the editor in `web/` beside it, for the command line, a server, or your
own browser. To build areas on your own computer, the app is simpler: each release also has an installer for Windows
(`-setup.exe`), macOS (`.dmg`) and Linux (`.AppImage`, `.deb`) that opens the editor in a window of its own
(<https://landscaper.layonara.com/handbook/getting-started/>).

You need Neverwinter Nights: Enhanced Edition, installed and started once (the first start makes your user folder,
`Documents/Neverwinter Nights`). The Landscaper reads the game's files and ships none of them.

## Start it

- **Windows:** double-click `landscaper.exe`. The program is signed, so SmartScreen sees a verified publisher. If it still says
  it protected your PC, click **More info**, then **Run anyway**.
- **macOS:** double-click `landscaper`. The program is signed and notarized by Apple, so macOS only asks the first time whether
  to open a program downloaded from the internet: click **Open**.
- **Linux:** run `./landscaper` in this folder.

A window opens and shows the game and the user folder it found, then your browser opens the editor at <http://127.0.0.1:8765/>
(or the next free port, if another program has 8765: the window says which). Keep the window open while you work. To stop
the Landscaper, close the window or press Ctrl+C in it. Only this computer can reach the editor.

## If it can't find the game

The window says so, and what to set. The game's folder is the one with `data/nwn_base.key` in it. Set `NWN_ROOT` to it (and
`NWN_HOME` to your user folder, if that isn't found either), then start the Landscaper again. On Windows, in a Command Prompt:

```text
setx NWN_ROOT "D:\Games\Neverwinter Nights"
```

On macOS add `export NWN_ROOT="/path/to/Neverwinter Nights"` to `~/.zshrc`, on Linux to `~/.profile`.

If you have used the app, the program also finds the folders you gave it there (`settings.json` in the data folder).

## More

- Keep `web/` next to the program: it is the editor.
- Exports go into your user folder's `hak/` and `modules/`.
- **Package for your module…** needs `nwn_gff` from neverwinter.nim (<https://github.com/niv/neverwinter.nim/releases>) on
  your `PATH`.
- From a terminal, `landscaper --help` lists the commands; `landscaper serve` starts the editor without opening the browser.
- The handbook: <https://landscaper.layonara.com/>. The source: <https://github.com/plenarius/nwn-landscaper>.
