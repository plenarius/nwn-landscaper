<!-- markdownlint-disable-file MD041 -->
- **On Windows, a file being replaced no longer fails its reader.** Windows can refuse to open a file ("Access is denied") at
  the moment the Landscaper replaces it, for example the texture catalog while a pack installs, or an area while it saves, so
  the request reading it failed once. Reads of the files the Landscaper replaces now try again for up to a second, and so
  does the replace itself while another program has the file open. Publishing a pack no longer starts a new pack list when
  the old one couldn't be read, which dropped every other pack from it.
