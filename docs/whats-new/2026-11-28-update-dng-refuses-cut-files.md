### Update DNG refuses a DNG that was cut short
- A DNG cut short by an interrupted copy (or one whose raw data claims more bytes than the file holds) was "updated", and the new preview and metadata landed inside its raw data. Such a file is now refused with the reason and left as it was.
