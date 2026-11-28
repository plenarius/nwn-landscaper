# Getting started

Install the Landscaper, make your first area and find your way around the view.

## Install the Landscaper

From 0.7.0 the Landscaper is an app: install it, start it, and the editor opens in a window of its own with the engine inside
it. Take the download for your system from the [releases](https://github.com/plenarius/nwn-landscaper/releases):

| System | Download | Page |
| --- | --- | --- |
| Windows 10 and 11 | `nwn-landscaper-0.7.0-windows-x86_64-setup.exe` | [Windows](install-windows.md) |
| macOS 11 and newer | `nwn-landscaper-0.7.0-macos-universal.dmg` | [macOS](install-macos.md) |
| Linux | `nwn-landscaper-0.7.0-linux-x86_64.AppImage`, or `nwn-landscaper_0.7.0_amd64.deb` | [Linux](install-linux.md) |

You need the game itself, Neverwinter Nights: Enhanced Edition. The Landscaper reads its files and ships none of them. The first
start looks for the game, asks for its folder when it can't find it, and builds the texture library from it:
[The first start](first-start.md).

Two other ways in:

- **[The program alone](program-alone.md):** the zip or tar archives, for the command line, a server, or your own browser.
- **[Try it in your browser](try.md):** the editor and its engine in a web page, nothing to install.

!!! note "No engine, sculpting only"
    When the page says *No engine: sculpting only*, the engine isn't answering. You can still shape the ground, and the editor
    keeps your area until the engine is back, but textures, import, export and the game's models need the engine. In the app,
    close it and start it again; with the program alone, start the program again.

## Make an area

**File > New area** (`N`) asks for:

- **Name:** 1 to 9 letters, digits or `_`. It starts every resref the area exports.
- **Module:** the game module it goes into. Leave it empty for a module of its own, or name another area's module to link the two
  (a cave beside its surface).
- **Size:** 3 to 32 tiles each way; a tile is 10 m.
- **Start from:** a card each, with a picture of what it makes; hover over one, or pick it, for a line on it. The arrow keys move
  between them. **Blank** is flat ground, gentle or rough hills, a cave or an interior (solid rock to cut passages or rooms
  out of). **Starter landscapes** are a canyon, a snowy mountain pass, a dune desert, a valley with a lake and a descending
  cave; they come painted, lit and with their weather, and come out different each time.
- **Start height** (flat ground and hills only): start high (a plateau) to dig valleys down into it.

![The New area dialog](../assets/handbook/new-area.webp)

**File > Open a showcase** opens a copy of one of the [showcase](../showcase.md)'s finished areas: the forest, swamp, plains,
lake valley, canyon, mountain pass, dunes or cave descent, to look round and change. The copy is a new area of yours, under the
showcase's name (`lsforest`) or, when you have an area of that name, the next free one (`lsforest2`); it never replaces an area.

**File > Open** (`O`) opens another of your areas, or imports one: from your module's source, or from a `.mod` or `.erf` you
upload. An imported area keeps its tileset's own textures, and its placeables, creatures, doors and scripts come with it on
export. A file in your areas folder that can't be opened (a name longer than 9 characters, a `name` that isn't its file's name,
or broken JSON) is listed greyed below your areas, with the reason.

The engine saves each change a moment after you make it. There is no Save button; **File > Save version** keeps a named copy in
a team's history.

## Move around

| Do | Mouse | Keys |
| --- | --- | --- |
| Sculpt or paint | left-drag | |
| Turn the view | right-drag | Shift+arrows |
| Move the view | middle-drag | arrows |
| Zoom | wheel | `+` `-` |
| Look straight down | | `Home` |
| Frame the area | | `F` |
| Brush size | | `[` `]` |
| Undo, redo | | Ctrl+Z, Ctrl+Y |

Each tool has a key, shown under its button. The status bar shows the position, height and tile under the cursor, and whether
creatures can walk there. **Walkable** (`W`), in the top bar beside the path check, tints ground too steep to walk in red, by
the rule the export uses.
