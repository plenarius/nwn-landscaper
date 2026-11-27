<!-- markdownlint-disable-file MD041 -->
- **An import whose note can't be read no longer passes as the builder's own area.** Each import keeps a note of the existing
  area it came from (`source.json`), which is what ties it to the builder's assignment. A note that couldn't be read, or
  didn't parse, was taken as "not an import", so the assignment check was skipped: a builder whose assignment had ended could
  still delete or export the area, and its publish went out as a new area instead of a change to the existing one. Now only
  a missing note means "not an import"; a damaged one stops the action with a message naming the file, and nothing changes.
