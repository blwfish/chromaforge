## Library

- **A non-finite value never drops a photo:** only the log refused an op holding NaN or infinity; the op stayed applied in memory, and the checkpoint or snapshot at close wrote it as `null`, so the next open skipped that photo with no message (a web library didn't open at all). `Catalog::apply` now refuses such an op before it changes anything, and the checkpoint and snapshot refuse to write one, keeping what was saved before. A NaN directly in an optional field (an aperture of 0/0) is still saved as "not known".
