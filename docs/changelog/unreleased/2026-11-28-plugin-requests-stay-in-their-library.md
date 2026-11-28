## Fixes

- **Relayed plug-in requests stay in their library:** requests from `program.end` hooks, jobs and worker export hooks reach the session's thread through one channel and carried no library: after a library switch, `metadata.setStandard`, `develop.setCrop`, `photo.virtualCopy`, `library.add` and catalog reads from a call started in library A were answered in library B, on its photos with the same ids. Each `RelayHost` now records the session's library generation, and `HostRequest::answer` refuses a request from another one.
