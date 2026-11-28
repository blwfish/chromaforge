## Library

- **Replaced files keep their permission bits:** every `safe_file` replace (XMP sidecars, Update DNG, settings files) wrote a new temp file and renamed it over the old one, so a 0600 or 0444 file came back 0644. The new file now gets the replaced one's permission bits (Unix; set-user-ID, set-group-ID and sticky are not carried over). Extended attributes (Finder tags), ACLs, owner and creation date are still not kept, and a hard link to the old file keeps the old content.
