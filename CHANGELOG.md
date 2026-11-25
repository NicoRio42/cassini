# Changelog

## 0.19.0

### Changed

- **Vegetation by height above the ground.** The low, medium and high vegetation layers are now sorted by the height of the points above the ground: up to 1 m, 1 m to 4 m and 4 m to 30 m. Only the unclassified and vegetation points (classes 1, 3, 4 and 5) count. The `--vegetation-layers lidar-class` option of `process`, `lidar` and `batch` keeps the previous layers, the LiDAR vegetation classes. The vegetation thresholds of `config.json` are unchanged, so the maps look different: you may want to retune them.
- **Half a meter DEM.** The DEM computed from the ground points has half a meter cells instead of one meter. A cell without ground points takes the mean of the ground points of its one meter block, so the contours stay about the same.
- **Faster LiDAR reading.** Only the coordinates and the classification of the points are decompressed from LiDAR HD's LAZ files.
- `batch --skip-lidar` reuses the filled DEM of each tile and its surroundings (`dem_with_buffer.tif`), which the LiDAR stage now writes, and the vegetation layers of the previous run, so it cannot be combined with `--vegetation-layers`. It stops, before deleting anything, if the LiDAR output of a tile comes from an earlier version: run a batch once without `--skip-lidar` after upgrading, and after adding or removing tiles.

### Added

- **Web UI.** `cassini ui` opens a local web page to select IGN LiDAR HD tiles on a map, by clicking them or by drawing a rectangle or a polygon. It downloads them with their DEMs into `in`, runs `batch` on them into `out`, and shows the generated map on top of the tiles. The page also edits the batch options and `config.json`. Pass `--port` to choose the port of the local server, and `--no-open` to not open the page in the default browser.
- The install scripts remove the `cassini` function that the installation guide used to have Docker users add to their `.bashrc` or PowerShell profile, which would otherwise keep running the Docker image instead of the installed executable. They keep a copy of each file they change, with a `.cassini-backup` extension.
- **IGN DEMs.** Put IGN's MNT LiDAR HD tiles next to the LiDAR files to use them instead of the DEM computed from the ground points, or pass one with `--dem` to `process` or `lidar`. See [Use IGN DEMs](https://cassini-map.com/guides/batch-mode#use-ign-dems). They give fewer cliffs, mostly isolated dots around the main cliff bands.
