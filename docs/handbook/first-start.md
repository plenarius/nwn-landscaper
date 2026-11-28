# The first start and the Help menu

What the app does the first time it starts, how to point it at the game, and where it keeps your work and its log.

## Finding the game

The Landscaper needs Neverwinter Nights: Enhanced Edition, and two of its folders:

- **The game's folder**, where it is installed: the one with `data/nwn_base.key` in it.
- **Your user folder**, where the game keeps `hak`, `modules` and `tlk`: `Documents/Neverwinter Nights`. The game makes it the
  first time it starts, so start the game once before the Landscaper.

The app looks for the game in Steam's libraries and in GOG's and Beamdog's folders, then for the user folder where the game
keeps it. When it finds both, it goes straight on.

## When it can't find them

The app shows its own page before the editor: *The Landscaper needs Neverwinter Nights: Enhanced Edition: where is it?*

1. Under **The game's folder**, click **Choose…** and pick the game's folder (the one with `data/nwn_base.key`), or type its path.
2. Under **Your user folder**, do the same for the user folder, if it isn't filled in already.
3. Click **Start**. If a folder is wrong, the page says what is missing and waits for another.

The app keeps the two folders in `settings.json` in [the data folder](#where-your-work-is), and finds them there from then on.
The program in the archive and the command line read that file too.

To change them later, choose **Area > Game folders…** in the editor. The same page opens with the folders it uses; saving other
folders restarts the Landscaper, and **Back to the editor** leaves them as they are.

## The first start

After the folders, the window says *Starting the Landscaper…* while the engine builds the texture library from the game's
files. That takes about a minute, and happens once. Then the editor opens:
[Getting started](getting-started.md#make-an-area) goes on from there.

If the engine stops, the page says why and where the log is.

## The Help menu

The editor in the app has a **Help** menu:

- **Handbook:** these pages, in your browser.
- **Open the data folder:** your areas, workspaces, texture library and exports, in your file manager.
- **Open the log:** what the engine wrote in this start, `landscaper.log` in the data folder. The start before is kept beside it
  as `landscaper.previous.log`. Send both when you report a problem.

Links to other sites open in your browser, not in the app's window.

## Where your work is

The data folder holds your areas, workspaces, the texture library, `settings.json` and the log:

| System | The data folder |
| --- | --- |
| Windows | `%LOCALAPPDATA%\landscaper` |
| macOS | `~/.local/share/landscaper` |
| Linux | `~/.local/share/landscaper`, or `landscaper` in `$XDG_DATA_HOME` |

The app, the program in the archive and the command line share it, so each opens the areas the others saved. Exports go into
your user folder's `hak` and `modules`, whichever you use.

Its `cache` folder keeps the ground pictures and shadows an export baked, so the next export of the same ground doesn't bake
them again: up to 1 GB of ground and 128 MB of shadows, each picture removed after 30 days unused. Deleting it is safe; the
next export bakes them again.

The editor's own preferences, which a browser keeps for each page, are the window's own in the app: it starts with the
defaults even if you used the editor in a browser before.
