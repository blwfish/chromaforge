### A change is never saved twice after a failed save
- When writing the library's change log failed and the save that rescued it then couldn't empty the log (a stalled network drive), the rescued changes were written again and repeated the next time the library opened: a doubled History step, or a photo added twice. They are now saved once.
