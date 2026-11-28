## Library

- **Exif text in Latin-1 is read as written:** the TIFF reader decoded ASCII values as UTF-8, lossily, so a Latin-1 "©" or "é" in Copyright, Artist or Image Description became U+FFFD in the catalog, and Update DNG wrote that into the original's XMP as the photo's rights. Valid UTF-8 is still read as UTF-8; every other byte is now read as the Latin-1 character it is. Photos imported before keep the replacement mark until Read Metadata.
