## Library

- **Update DNG checks every byte before it replaces the file:** "only over the file that was read" compared length and modification time, so a same-size edit within a volume's timestamp granularity, or by a tool that keeps the time (`exiftool -P`), was overwritten, and with no time available only the length was compared. The file is now compared with what was read byte for byte right before the rename (after the new file is written and read back), and a file moved or deleted meanwhile is never recreated.
