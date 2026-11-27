<!-- markdownlint-disable-file MD041 -->
- **The area list says which files it leaves out.** `/api/areas` has `unlisted` (`{ file, why }`) beside `areas`: each `.json`
  of the areas folder that can't be opened, with the reason (a bad name, broken JSON, no header, or a `name` that isn't its
  file's name), and the server's log says each once. A file copied in by hand used to vanish from Open without a word. A file
  whose `name` differs from its file name is no longer listed under that name, which opened another file or none.
