## Import, export and formats

- **`null` keeps a preset's export setting:** `app.export {preset, …}` (control channel, MCP, the CLI) treats a key passed as `null` as not given, so the preset's value stays. Before, `null` replaced it and validation then applied the default: `removeLocation: null` exported the location a private preset removes, and `metadata: null` exported all metadata.
