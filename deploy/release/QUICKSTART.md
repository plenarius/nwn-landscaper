# NWN Landscaper: quick start

You need Neverwinter Nights: Enhanced Edition, installed and started once (the first start makes your user folder,
`Documents/Neverwinter Nights`). The Landscaper reads the game's files and ships none of them.

## Start it

- **Windows:** double-click `landscaper.exe`. Windows SmartScreen may say it protected your PC, because the program isn't signed:
  click **More info**, then **Run anyway**.
- **macOS:** double-click `landscaper`. The program isn't signed, so macOS refuses it the first time: open **System Settings**,
  **Privacy & Security**, and click **Open Anyway** (on macOS 14 and older, right-click the program and choose **Open**). Or, in
  Terminal, run `xattr -dr com.apple.quarantine` followed by this folder's path.
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

## More

- Keep `web/` next to the program: it is the editor.
- Exports go into your user folder's `hak/` and `modules/`.
- **Package for your module…** needs `nwn_gff` from neverwinter.nim (<https://github.com/niv/neverwinter.nim/releases>) on
  your `PATH`.
- From a terminal, `landscaper --help` lists the commands; `landscaper serve` starts the editor without opening the browser.
- The documentation: <https://github.com/plenarius/nwn-landscaper>.
