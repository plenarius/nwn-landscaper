<!-- markdownlint-disable-file MD041 -->
- **A texture or sky catalog that can't be read is never written over.** A damaged or unreadable `catalog.json` was read as
  an empty library, and the next texture added or pack installed saved a catalog with only that in it: the library lost
  every other texture or sky. Only a missing catalog starts an empty library now; any other stops the change with a message
  naming the file, and lists of what is installed say so instead. A saved area that can't be read is no longer saved over
  without a backup, and an export no longer starts its entrance ids again when the last export's header doesn't read.
