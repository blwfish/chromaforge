### An export path with `..` through a missing folder no longer replaces an original

- An export path such as `missing/../IMG_1.jpg`, or an export folder such as `missing/..`, named a photo's original once the export created `missing`, and the export replaced it. Such paths are now refused for every export, with a message asking for a path without `..`.
