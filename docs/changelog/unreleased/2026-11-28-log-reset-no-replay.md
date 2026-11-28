## Library

- **A rescued change is not replayed twice:** after a catalog log write failed, the snapshot that saved the lost and queued changes could be durable while resetting the log after it failed; `persist` then kept the queued changes and appended them again under new numbers, so the next open replayed them a second time (a duplicate History step; a duplicate added photo counted only as a failure). `persist` now checks, as closing the library already did, whether the snapshot holds them, and drops them from the queue when it does.
