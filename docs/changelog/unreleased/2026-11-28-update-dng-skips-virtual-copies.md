## Library

- **Update DNG skips virtual copies:** a virtual copy shares its master's file, and Update DNG Preview & Metadata rewrote that file with the copy's XMP and render (once per selected id, the last winning); the next Read Metadata or sidecar-less import then put the copy's edit on the master. Copies are now skipped with the reason, the command is unavailable for a selection of copies only, and a file named by several photos is rewritten once.
