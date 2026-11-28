## Import, export and formats

- **A `..` through a missing folder no longer replaces an original:** an exact export path such as `<folder>/missing/../IMG_1.jpg`, or an export folder `<folder>/missing/..` with `{name}`, passed the originals check (a path that isn't there looked free), then the export created `missing` and replaced the catalogued original, under Overwrite and Unique alike. The originals check now refuses a `..` that can't be resolved yet everywhere it is used, not only in `export.checkTarget` (#1635).
