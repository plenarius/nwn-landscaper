<!-- markdownlint-disable-file MD041 -->
- **An export keeps its baked ground and shadows on disk.** Each tile's baked ground texture and shadow map is kept in the data
  folder's `cache` folder, named by a digest of everything it is made from and of the program's own source, so the command line,
  a restarted `landscaper serve` and the browser page reuse them. Exporting the forest showcase again took 3 min 5 s instead
  of 6 min 53 s (408 s of CPU instead of 1,611 s, on 6 threads) and wrote the same hak; its 256 ground textures and shadow maps
  take 62 MB. The cache
  keeps up to 1 GB of ground and 128 MB of shadows (96 MB and 16 MB in the browser), and removes a file unused for 30 days. A
  shadow's key now holds its objects' models and how their textures let light through, so a hak's changed model is drawn again.
