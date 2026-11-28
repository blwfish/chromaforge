## Library

- **Deletions after an upgrading open are saved:** opening a library an older build wrote (from the CLI, MCP or the control channel) reopened its store without what had been loaded, so the first save didn't remove photos, albums or stacks deleted after the open (or by the log it replayed). They came back on the next open, and the integrity test then failed at every exit, so no scheduled backup was made. The loaded store is now kept across the upgrade, which also keeps the count of records the load couldn't read, so Optimise stays refused there.
