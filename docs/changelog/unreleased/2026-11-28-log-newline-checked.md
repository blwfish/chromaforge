## Library

- **The open's newline fix-up is checked again:** a log cut right after a record's bytes gets its newline at open. Since the log has its own writer thread that byte was handed over and counted as written without waiting; if its write failed, the next record was glued onto the last (`…}{"seq":…`) and the next open dropped both, an acknowledged change included. The open now waits for the byte to be synced and refuses with an actionable error when it couldn't be written (as it did before the log thread).
