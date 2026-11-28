## Library

- **Update DNG blanks what it replaces:** when the new XMP packet, IFD0 or preview didn't fit where the old one was (the usual first update), it was appended and the old packet, IFD0, SubIFDs list and previews of the app's stayed in the file unreferenced, so a cleared GPS position could still be read by scanning the bytes. They are now zeroed, never touching a byte the file still references.
