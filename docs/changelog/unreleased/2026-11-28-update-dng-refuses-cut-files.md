## Library

- **Update DNG refuses a file whose data runs past its end:** the preview, XMP and IFD0 are appended at the end of the file; when a strip, tile or value the file references ran past that end (a copy cut short, a wrong byte count), the new bytes landed inside it, and the read-back check clipped every protected range to the old length and so excused exactly those bytes. Such a file is now refused (by the check before rendering too), and the read-back no longer clips.
