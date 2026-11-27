# The program alone

Each release still has the program on its own, beside the installers: a zip for Windows and a tar archive for macOS and Linux,
with the engine and the editor in `web/` next to it. Use it for the command line, to run the Landscaper on a server, or to use the
editor in your own browser. To build areas on your own computer, [the app](getting-started.md#install-the-landscaper) is simpler.

| System | Download |
| --- | --- |
| Windows | `nwn-landscaper-0.7.0-windows-x86_64.zip` |
| macOS | `nwn-landscaper-0.7.0-macos-universal.tar.gz` |
| Linux | `nwn-landscaper-0.7.0-linux-x86_64.tar.gz` |

## Start it

1. Unpack the archive anywhere. Keep `web/` next to the program: it is the editor.
2. Start `landscaper.exe` (Windows) or `landscaper` (macOS, Linux). A terminal window shows the game and the user folder it
   found.
3. Your browser opens the editor at `http://127.0.0.1:8765/` (or the next free port, which the window names). Keep the window
   open while you work; close it, or press Ctrl+C in it, to stop the Landscaper.

The Windows program is signed, so SmartScreen sees a verified publisher. The macOS program is signed and notarized: the first
start asks whether to open a program downloaded from the internet, and needs to reach Apple's server once.

When it can't find the game, the window says what to set: `NWN_ROOT` for the game's folder and `NWN_HOME` for your user folder.
It also reads the folders [the app](first-start.md) saved, if you used it. `QUICKSTART.txt` in the archive has the details.

## The command line and servers

`landscaper --help` lists the commands: `serve` (the editor, without opening a browser), `import`, `export`, `check`,
`textures` and the rest, described in the
[README](https://github.com/plenarius/nwn-landscaper#use). To run it for a team on a server of your own, see
[On your machine or a server](../SELF-HOSTING.md) and [The team server](../TEAM-SERVER.md).

The program, the app and the command line share the same data folder ([Where your work is](first-start.md#where-your-work-is)).
