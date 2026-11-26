- **The desktop app** (`app/`, step 2 of `docs/DESIGN-desktop-app.md`): the editor in a window of its own, with the engine inside
  it on 127.0.0.1:8766 (else the next free port up to 8785). Only one copy runs; a second start brings its window forward. When it
  can't find the game, its first page asks for the game's folder and the user folder and keeps them in `settings.json`. What it
  prints goes to `landscaper.log` in the data folder. Links to other sites open in the default browser, and downloads go to the
  Downloads folder. On Linux it turns off WebKitGTK's DMA-BUF renderer under NVIDIA's driver, and `LANDSCAPER_WEBVIEW_SAFE=1`
  turns off compositing as well. Not in the release yet: the installers come with step 4.
