### A script's empty values keep an export preset's privacy settings
- An export started by a script, an agent or the command line with a preset now keeps the preset's setting for any value passed as `null`. Before, `null` reset it to the default, so a preset's Remove Location Info or Metadata choice could be switched off without anyone asking.
