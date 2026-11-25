# Plan: 0.5 m DEM, IGN DEM input and vegetation by height above ground

Date: 2026-11-25

## Goal

This plan replaces step 8 of [the PDAL/GDAL rewrite plan](2026-11-24-pdal-gdal-rust-rewrite.md). Steps 1–7 of that plan are done: Cassini runs without external tools. Its "Notes from completed steps" stay the reference for the tools, the reference outputs and the semantics this plan builds on.

Three changes, each one changing the render, so each is measured on its own:

1. **A 0.5 m DEM from the ground points** instead of 1 m, because a finer DEM finds more cliffs.
2. **IGN DEMs as input.** IGN publishes a filled 0.5 m DEM computed from LiDAR HD (MNT LiDAR HD). The next mapant.fr map will use it. Batch users should only have to put the DEM files next to the LAZ files.
3. **Vegetation layers from height above ground**, as the PDAL pipeline meant to and never did (step 4 notes of the old plan), with the current class-based layers kept as an option.

## How to use this plan

- **One step per Claude Code session.** Start a session with: *"Implement step N of `specs/plans/2026-11-25-dem-ign-and-vegetation-height.md`."*
- **Branches:** each step works on its own branch from `develop` (`dem-vegetation/step-N-<slug>`) and is merged before the next step starts.
- **Order:** 1 → 2 → 3, sequential. Step 3 takes its heights from whichever DEM steps 1 and 2 produce.
- **At the end of each session:**
  - Update the status table below.
  - Append any finding the next steps need to "Notes from completed steps" at the bottom: decisions, measured differences, timings, surprises.
  - Keep notes short. This file is the hand-over between sessions.
- **Builds:** always use `cargo build --locked` / `cargo install --path . --locked`.
- **Downloads** (IGN DEM tiles) need the user's OK in chat first: say which files, from where, and their size.

## Status

| Step | Title | Depends on | Status |
|---|---|---|---|
| 1 | 0.5 m DEM from the ground points | – | Done |
| 2 | IGN DEM input (`lidar`, `process`, `batch`) | 1 | Done |
| 3 | Vegetation layers from height above ground | 2 | Done |

## Shared context

### Who uses which command

- **Users run `batch`** (and sometimes `process`). These are the commands to design for.
- **`lidar` and `render` exist only for mapant.fr.** The current map generator is the CLI in `/home/nicolas/Documents/sources/mapant`. It runs `cassini render <tile> -C cassini.json --skip-520 --undergrowth 409 --shapefiles <dir> -o <dir> -n <neighbour> …` on LiDAR-step directories. `mapant-fr-worker` is the older pipeline. **No backward compatibility is needed for either:** the user updates mapant to follow the CLI. Record each CLI change in the notes so mapant can be updated.

### Today's DEM chain

- **`dem.tif`** (in [lidar.rs](../../src/lidar.rs)): mean Z of the ground points (class 2) per **1 m** cell over exactly the tile, using PDAL's running mean. Points are binned with `TileRasters::bin_index`, `floor(x - min_x)`, `floor(y - min_y)`: a point on a cell edge goes to the east or north cell, and points on the tile's east and north edges are dropped. Float64, Deflate, nodata -9999.
- **`dem_with_buffer.tif`** ([buffer.rs](../../src/buffer.rs), [dem.rs](../../src/dem.rs)): the `dem.tif` of the tile and its neighbours, pasted with nearest neighbour onto a 0.5 m grid (`DEM_BLOCK_SIZE`) covering the tile plus 200 m (`BUFFER`), then filled with `fill_nodata` (100 cells). Float32, uncompressed. Today each 1 m cell becomes four equal 0.5 m cells.
- **From the filled DEM:** `dem_2m_with_buffer.tif` (4×4 average) → contours every 2.5 m (`contours-raw/contours-raw.shp`, then Pullautin smoothing) and `slopes.tif` (Horn, 0.5 m) → cliffs.
- **Vegetation rasters** stay at 1 m in every step of this plan: `low/medium/high_vegetation.tif`, Byte, nodata 255, counts after 0.5 m voxel thinning.
- `paste_nearest` works in world coordinates: an output cell takes the source cell that contains its centre (`floor`). It accepts any source resolution and origin.

### Ground point density

On tile 907000_6454000 there are 6.24 ground points/m². 83.4% of 1 m cells contain at least one ground point, but only 59.7% of 0.5 m cells do. At 0.5 m, about 40% of the cells are left for `fill_nodata`, against 17% at 1 m.

### IGN DEM tiles

Example download (WMS GetMap from the Géoplateforme):

```
https://data.geopf.fr/wms-r?SERVICE=WMS&VERSION=1.3.0&EXCEPTIONS=text/xml&REQUEST=GetMap&LAYERS=IGNF_LIDAR-HD_MNT_ELEVATION.ELEVATIONGRIDCOVERAGE.LAMB93&FORMAT=image/geotiff&STYLES=&CRS=EPSG:2154&BBOX=661999.75,6708000.25,662999.75,6709000.25&WIDTH=2000&HEIGHT=2000&FILENAME=LHD_FXX_0662_6709_MNT_O_0M50_LAMB93_IGN69.tif
```

- For the tile `min_x, min_y, max_x, max_y`: `BBOX = min_x - 0.25, min_y + 0.25, max_x - 0.25, max_y + 0.25`, 2000×2000, `FILENAME = LHD_FXX_<min_x/1000, 4 digits>_<max_y/1000, 4 digits>_MNT_O_0M50_LAMB93_IGN69.tif`, like the LAZ names.
- **The grid is offset by half a cell from Cassini's.** IGN cell centres are on multiples of 0.5 m (662000.0, 662000.5, …) and its edges on .25/.75. Cassini's 0.5 m cells have their edges on multiples of 0.5 m and their centres on .25/.75.
- **Decision (user): nearest neighbour, no resampling.** Every Cassini cell centre falls exactly on an IGN cell edge. `paste_nearest`'s `floor` sends the tie to the IGN cell east (x) and south (row below) of it, so the DEM appears **0.25 m west and 0.25 m north** of its true position (0.35 m, 0.035 mm at 1:10 000). This is exact in `f64` (all values are multiples of 0.25) and therefore deterministic. Bilinear resampling would average 2×2 cells and soften cliffs. Re-aligning Cassini's grids to IGN's would touch every consumer of the buffered rasters.
- **Coverage:** an IGN tile misses the 0.25 m strips on the east and south of its Cassini tile and overhangs on the west and north. With the rule above, the tile's last column and last row come from the east and south neighbours' IGN tiles. On a lone tile they stay nodata and are filled.
- **Not checked yet:** data type, nodata value, PixelIsArea or PixelIsPoint, and which GeoKeys the file carries (a vertical CRS for IGN69, for instance). `Grid::read_geotiff` checks only `ProjectedCSTypeGeoKey` (3072) and honours PixelIsPoint. Step 2 starts by checking a real file.
- mapant's `lidar fix` already downloads these tiles and puts them in place as `dem.tif` (quantized to 2 decimals), so `render` already runs on IGN DEMs.

### Test data and baseline

- **Inputs:** `in-poussebout/` (four LiDAR HD tiles), `in-arselle`, `in-correncon`, `in-evg`, `in-tunnels`. `in*` and `out*` are git-ignored.
- **`out-step6-<dataset>`** holds full-batch outputs (LiDAR included, `--ocad`) of the step 6 binary for all five datasets, including their `.osm` files. Step 7 changed nothing in the render.
- **Tools** (see the old plan's notes for details): `compare_rasters`, `compare_images`, `compare_shapefiles`, `osm_to_shapefiles`, `crop_laz` in `examples/`. `compare_rasters` needs aligned cells of the same resolution.
- **Baseline for this plan, built at the start of step 1** from `develop`, so every step can be compared with the state before the plan:

  ```bash
  git switch develop
  cargo build --release --locked
  mkdir -p out-baseline && cp target/release/cassini out-baseline/cassini-baseline
  for d in poussebout <second dataset>; do
    cp -r out-step6-$d out-baseline-$d     # keeps the .osm files, so nothing is downloaded
    ./out-baseline/cassini-baseline batch -o out-baseline-$d --ocad in-$d
  done
  ```

  The second dataset is chosen in step 1: the one among arselle, correncon, evg and tunnels with the most cliffs (cliff pixels in its `cliffs.png`). Record the choice and use the same two datasets in every step.

### Before/after review for the user

The user judges render changes by eye, so every step produces **side-by-side crops**: before | after | `compare_images` diff, for `contours.png`, `cliffs.png` and `full-map.png`. Use about 3 areas of roughly 250 m per dataset, picked for steep ground, flat ground and a tile edge. Save them in the session scratchpad and send them to the user with `SendUserFile`. A small example (for instance `examples/crop_side_by_side.rs`) is fine if it helps.

### Thresholds

Leave every vegetation and cliff threshold in [config.rs](../../src/config.rs) unchanged in all three steps. The user retunes them after this plan, from the numbers the steps record.

---

## Step 1 — 0.5 m DEM from the ground points

**Goal:** compute `dem.tif` at 0.5 m instead of 1 m, and show its effect on cliffs and contours.

**Tasks**

1. **Baseline:** build it as described in "Test data and baseline" and choose the second dataset.
2. **DEM at 0.5 m** in [lidar.rs](../../src/lidar.rs):
   - Bin ground points into `DEM_BLOCK_SIZE` cells over exactly the tile (2000×2000 for a 1 km tile). Use the same rule as `bin_index`, `floor((x - min_x) / 0.5)`, with east/north edge points going east or north. A 1 m vegetation cell is then exactly 2×2 DEM cells, and a point's DEM cell lies inside the vegetation cell it is counted in.
   - Keep PDAL's running mean in `f64`.
   - `TileRasters` needs one binning per resolution: 0.5 m for the DEM, 1 m for vegetation. Keep the vegetation path untouched.
   - Write `dem.tif` as **Float32**, Deflate, nodata -9999. Every consumer reads it as `f32` (`mosaic_with_buffer::<f32>`), and IGN DEMs are Float32, so nothing downstream changes from the narrowing. It also halves the larger file.
3. **Buffer step:** with a 0.5 m `dem.tif`, `paste_nearest` copies cells one to one. A 1 m `dem.tif` left by an older run (for `--skip-lidar`) still upsamples as before. Add a test for the one-to-one case.
4. **Measure**, on the two datasets, against `out-baseline-<dataset>`:
   - Share of 0.5 m cells with a ground point, and of cells filled by `fill_nodata`.
   - Fill time per tile (more holes than before), and LiDAR stage time per tile.
   - **Cliffs:** cliff pixels in `cliffs.png`, and the number of cliff objects in the OCAD export, before and after.
   - **Contours:** total length of `contours-raw.shp` per level and in all, feature count, and the same for the smoothed `contours/contours.shp`. Write a small example for this if no tool does it (`compare_shapefiles` only handles the OSM layers). The user expects contours to stay roughly the same, so call out any level whose length changes by more than a few percent.
   - Before/after crops of `contours.png`, `cliffs.png` and `full-map.png`, sent to the user.
5. Docs: update any page that says the DEM is 1 m.

**Acceptance:**
- Vegetation rasters byte-identical to the baseline (`compare_rasters` exits 0).
- `dem.tif` is 0.5 m, Float32, exactly the tile.
- Cliff and contour numbers recorded, and the crops shown to the user, who judges whether the contours stay close enough.
- `cargo test` is green.

---

## Step 2 — IGN DEM input

**Goal:** use an IGN DEM instead of computing the DEM from the points: given explicitly to `lidar` and `process` (optional for both since the step 2 review), and found in the input directory by `batch`.

**Tasks**

1. **Check a real IGN file first.** With the user's OK, download the tiles covering `in-poussebout` (4 files, about 16 MB each if uncompressed) into a new git-ignored `in-poussebout-ign/` that also holds the four LAZ files (symlinks are fine). Record the data type, nodata value, PixelIsArea/PixelIsPoint, GeoKeys and extent from `gdalinfo`. Make `Grid::read_geotiff` accept the file if it doesn't yet (an extra vertical CRS key, say), without loosening the Lambert-93 check.
2. **Reading an external DEM** (a new module, e.g. `src/external_dem.rs`):
   - Read it as `Grid<f32>` in Lambert-93. Accept any square cell size (tested with 0.5 m); the mosaic samples it with nearest neighbour onto Cassini's 0.5 m grid.
   - Write it to the LiDAR output directory as `dem.tif` **on its own grid**, unchanged apart from Float32 and Deflate (user's decision: the DEM goes in the output). Do not resample it onto the tile grid: the strips on the east and south of the tile come from the neighbours' DEMs at the mosaic step, and resampling each DEM onto its own tile would lose them.
   - Error if it doesn't overlap the tile. Warn if it leaves more than one cell's width of the tile uncovered.
3. **Commands:**
   - `lidar --dem <file>`: **required** (optional since the step 2 review). The points are read for the vegetation only; the ground DEM is not computed.
   - `process --dem <file>`: optional. Without it, the DEM comes from the points as in step 1.
   - `render`: unchanged. It mosaics and fills the `dem.tif` of the tile and its `-n` neighbours, whatever their source.
4. **`batch` finds the DEMs in its input directory:**
   - Read the GeoTIFF extent of every `.tif`/`.tiff` file next to the `.laz` files (case-insensitive extension). Match each DEM to the LAZ tile that contains its extent's centre, not by file name, so renamed files still work.
   - Two DEMs for one tile: error. A DEM matching no tile: warning, ignored. A DEM that fails to read or isn't Lambert-93: error naming the file (a warning since the step 2 review).
   - Tiles with a DEM skip the ground DEM accumulation. Tiles without one compute it from the points. A batch mixing both works, with one warning listing the tiles without a DEM: seams between the two kinds of DEM may show in the contours.
   - `--skip-lidar` uses the `dem.tif` of the previous run, as now.
5. **Docs:** how to download IGN DEM tiles (the URL template above), dropping them in the input directory, `--dem` on `lidar`/`process`, and the 0.25 m shift. CLI reference. Record the new `lidar` CLI in the notes for mapant.

**Acceptance:**
- On `in-poussebout-ign`, a full batch runs with `PATH=/nonexistent`, and its `dem.tif` files are the IGN tiles (same values and extent as the downloads).
- `dem_with_buffer.tif` from IGN DEMs against the step 1 one from the points: mean / p99 / max absolute difference over cells valid in both, recorded. Both are on Cassini's grid, so `compare_rasters` applies.
- The tile's last column and last row come from the neighbours' IGN tiles in a batch (unit test of the tie rule on a half-cell-offset source).
- Before/after crops (step 1 points DEM vs IGN DEM) of contours, cliffs and the full map, shown to the user, with the same cliff and contour numbers as step 1.
- Mixed batch (IGN DEMs for some tiles only) and `lidar` without `--dem` behave as specified. Vegetation rasters unchanged from step 1.
- `cargo test` is green.

---

## Step 3 — Vegetation layers from height above ground

**Goal:** classify vegetation by height above ground, and keep the current class-based layers as an option. Also decode only the point fields Cassini needs, so the extra pass costs little.

**Tasks**

1. **Option** `--vegetation-layers <height|lidar-class>` on `batch`, `process` and `lidar`, default `height`, a clap `ValueEnum` like `--undergrowth`. `lidar-class` is today's behaviour. Not in `config.json` for now. It is independent of the DEM source.
2. **Batch order** (both modes, so `render` sees the same files):
   - For all tiles, in parallel: the DEM (`dem.tif`, from the points or the IGN DEM). In `lidar-class` mode the same pass also computes the vegetation rasters, as now.
   - Barrier: the next stage needs every neighbour's `dem.tif`.
   - For each tile with its neighbours: mosaic, buffer and fill the DEM, and write `dem_with_buffer.tif`. This is the first stage of today's render step, moved earlier.
   - For all tiles, in `height` mode: the vegetation pass, with heights from the tile's filled DEM. With an IGN DEM, this is the only pass over the points.
   - The batch render stage reads `dem_with_buffer.tif` instead of rebuilding it. Split [dem.rs](../../src/dem.rs) into "prepare the filled DEM" and "2 m DEM, contours and slopes from it".
   - Remove `dem_with_buffer.tif` from `RENDER_STEP_FILES` in [render.rs](../../src/render.rs), since `--skip-lidar` would delete it. `--skip-lidar` needs the file from a previous full run, with a clear error if it is missing. `--vegetation-layers` has no effect with `--skip-lidar`. A `--skip-lidar` run after tiles were added or removed reuses buffers built with the old neighbours: say so in the docs.
3. **The `render` command** keeps building and filling the buffered DEM from the tile's and the neighbours' `dem.tif` (mapant runs `lidar` and `render` separately).
4. **Single-tile commands** (`lidar`, `process`) have no neighbours: paste the tile's DEM onto its 0.5 m buffered grid without neighbours, fill it, and take heights from it in memory. `lidar` doesn't write `dem_with_buffer.tif`, since mapant uploads the LiDAR directory. With a DEM from the points, cells near the tile edge can differ from a batch run; say so in the docs. With an IGN DEM, only the 0.25 m strips (filled) differ.
5. **`height` mode semantics:**
   - Voxel thinning first, on every point, as now (same origin: the first point read, minus half a voxel).
   - Only classes 1, 3, 4 and 5 (unclassified and vegetation) can be vegetation. Ground (2), buildings (6), water (9), bridges (17) and the other LiDAR HD classes never are.
   - Height above ground = Z − the filled DEM at the 0.5 m cell containing the point on Cassini's grid, with the same `floor` rule as the binning (edge points go east or north). Not `Grid::cell_at`, which sends edge points south: LiDAR HD coordinates are on a 1 cm grid, so about 1% of points lie on a y edge.
   - Layers: (0, 1] low, (1, 4] medium, (4, 30] high, counted in the 1 m cell of the point as now.
   - A point outside 0–30 m, or on a cell still nodata after filling, is dropped. It does not fall back to its LAS class.
6. **Selective decompression:** LiDAR HD uses LAS 1.4 layered point formats. Decode only X, Y, Z and classification:
   - Use `laz::ParLasZipDecompressor::selective` with `DecompressionSelection` (laz 0.13; `las` doesn't expose it). Add `laz` as a direct dependency at the version `las` already uses. Get the LAZ VLR with `las::Header::laz_vlr()`, and the scales and offsets from the header.
   - Parse the raw records: X, Y, Z are `i32` at bytes 0, 4, 8. Classification is byte 16 for formats 6–10 and the low 5 bits of byte 15 for formats 0–5 (where selection is ignored and everything is decoded). Uncompressed `.las` files keep going through `las::Reader`.
   - Points must come in the same order as now, so the voxel origin and every `lidar-class` output stay identical. Decompression stays on rayon's global pool.
   - Use it in every pass and in both modes. Record the gain: two full passes would otherwise roughly double the LiDAR time (10.9 s against 5.4 s on tile 907000_6454000).
7. **Thresholds:** unchanged (see "Thresholds"). Record the mean points per cell per layer in both modes and with both DEM sources. With PDAL's broader class rule and the 1 m DEM, tile 907000_6454000 went from 1.01 / 0.68 / 12.7 (low / medium / high) to about 1.2 / 2.8 / 10.4.
8. **Release:**
   - Create `CHANGELOG.md` (dist writes the GitHub release notes from it) with an entry covering all three steps.
   - Bump the version to 0.19.0 (0.18.0 is released, although `origin` has no 0.18.0 tag).
   - CLI reference and the single-tile guide.

**Acceptance:**
- `--vegetation-layers lidar-class` gives vegetation rasters and final renders identical to the step 2 outputs, with both DEM sources.
- In `height` mode, `dem.tif`, `dem_with_buffer.tif`, contours and cliffs are identical to `lidar-class` mode.
- `height` mode renders on poussebout and the second dataset, with both DEM sources, are shown to the user with the per-layer densities of both modes, so they can retune the thresholds later. They are not expected to match `lidar-class` renders.
- LiDAR stage time per tile recorded for both modes, before and after selective decompression.
- `cargo test` is green.

---

## Notes from completed steps

_Each session appends findings here: decisions, measured differences, timings, surprises._

### Step 1 (branch `dem-vegetation/step-1-dem-05m`, cut from `develop`)

- **Baseline:** `out-baseline/cassini-baseline` (`develop` at 7f8914a). **Second dataset: arselle** (3.22 M cliff pixels in `cliffs.png`, against 1.93 M for evg, 0.53 M for tunnels, 0.42 M for correncon). Baseline outputs are `out-baseline-poussebout` and `out-baseline-arselle`, step 1 outputs `out-dem-step1-<dataset>` (full batch, `--ocad`, from copies of `out-step6-<dataset>`). The step 1 binary is `out-baseline/cassini-dem-step1`.
- **Code:** [lidar.rs](../../src/lidar.rs) bins with a `TileCells` per resolution (`DEM_BLOCK_SIZE` for the DEM, `VEGETATION_BLOCK_SIZE` for vegetation), same `floor` rule; a compile-time check makes a vegetation cell a whole block of DEM cells, and a test checks a 1 cm point grid lands in matching cells. `dem.tif` is Float32, Deflate, nodata -9999, 2000×2000 for a 1 km tile; the mean stays `f64` until written. [dem.rs](../../src/dem.rs) now logs the fill: cells filled, cells without data, total, and fill time.
- **No CLI change**, nothing for mapant to update. `dem.tif` doubles in size (3.8 → 7.7 MB on tile 907000_6454000).
- **Acceptance:** vegetation rasters byte-identical to the baseline on all 8 tiles (`compare_rasters` exit 0). A `--skip-lidar --ocad` batch of the new binary on the baseline's 1 m `dem.tif` files gives pixel-identical PNGs (the old-run path still upsamples). `cargo test` green.
- **Tools:**
  - `cargo run --release --example measure_dem_render -- out-a [out-b] [--level-threshold 5]`: per tile, the share of 0.5 m cells with a ground point / filled, the cliff pixels of `cliffs.png`, the contour length and line count per level and in all (`contours-raw.shp` and `contours.shp`, clipped to the tile), and the 202 / 205 objects of `merged-map.ocd`. With two directories it prints the changes and the levels that change by more than the threshold.
  - `cargo run --release --example crop_side_by_side -- out-a out-b crop.png --crop min_x,min_y,max_x,max_y`: rows `contours.png`, `cliffs.png`, `full-map.png`; columns a | b | diff. The crop may span tiles.
  - Crop areas (250 m): poussebout steep `907375,6455025,907625,6455275`, flat `907275,6455725,907525,6455975`, tile corner `907875,6454875,908125,6455125`; arselle steep `927675,6448725,927925,6448975`, flat `926075,6448175,926325,6448425`, tile corner `926875,6448875,927125,6449125`. Reuse them in steps 2 and 3.
- **Ground cells inside the tile** (with a point / filled), 1 m DEM → 0.5 m DEM:
  - poussebout: 83.4/16.6 → 59.7/40.3, 67.0/33.0 → 48.7/51.3, 54.8/45.2 → 31.0/69.0, 73.2/26.8 → 42.5/57.5 %
  - arselle: 85.1/14.9 → 75.9/24.1, 84.3/15.7 → 75.4/24.6, 73.3/26.7 → 53.7/46.3, 88.5/11.5 → 77.7/22.3 %
- **Timings** (4 tiles, default 3 threads, 12 cores, warm cache):
  - LiDAR stage per tile: poussebout 5.9–7.0 s → 6.4–7.2 s, arselle 4.7–6.4 s → 5.4–7.1 s (+0.3–0.7 s, the larger DEM arrays and file).
  - Fill per tile: 0.35–0.51 s (1 m `dem.tif`, measured with the new binary's log on the baseline's files) → 0.46–0.77 s. Mosaic + fill + write: 0.45–0.62 s → 0.73–0.96 s.
  - Full batch: poussebout 20.5 → 21.7 s, arselle 19.5 → 22.1 s.
- **Cliffs go down, not up:**

  | | cliff pixels | 202 objects | 205 objects |
  |---|---|---|---|
  | poussebout | 2 095 035 → 1 585 639 (−24.3%) | 2348 → 1905 (−18.9%) | 193 → 97 |
  | arselle | 3 216 357 → 2 928 628 (−8.9%) | 3219 → 3210 (−0.3%) | 107 → 35 |

  Per tile, −0.0% to −44.7%: the sparser the ground points, the larger the drop. The loss is in the cells without a ground point at 0.5 m, which `fill_nodata` interpolates. Cells steeper than the 60° threshold, poussebout 908000_6455000: 1 897 → 1 802 with a point, 19 749 → 12 129 without; 907000_6454000: 100 764 → 101 317 with, 89 212 → 71 618 without; arselle 927000_6448000: 398 508 → 409 593 with, 257 709 → 220 294 without. In the crops, the cliffs lost are mostly isolated dots around the main bands; the bands stay. Likely cause, not checked: the 1 m DEM held each mean over a 2×2 block of slope cells, while at 0.5 m more and wider gaps are left to the inverse-distance fill, which smooths them. IGN's filled DEM (step 2) changes this again.
- **Contours stay close.** Total length clipped to the tiles, `contours-raw.shp`: −0.4% (poussebout), −0.3% (arselle); lines −2.8% / −1.9% (fewer short closed loops). `contours.shp`: −0.2% / −0.1%, lines −0.9% / −0.6%. Per-level change, median 0.40% / 0.28% raw, 0.15% / 0.10% smoothed. Levels over 5%: only tiny ones (poussebout raw 1680 m: 14 → 16 m; arselle 712.5 m: 4 → 2 m raw, 4 → 0 m smoothed, and 720 m: 54 → 59 m smoothed). Crops show the same lines with sub-pixel shifts (1.1–2.6% of `contours.png` pixels differ on steep and edge crops, 0.25–1.4% on flat ones).
- **Fallback added after review (supersedes the cliff drop above):** a 0.5 m cell without a ground point now takes the mean of the ground points of its 1 m block (`FALLBACK_BLOCK_SIZE`, weighted by the cell counts, so the old 1 m DEM value), in `TileRasters::dem` in [lidar.rs](../../src/lidar.rs). Only 1 m blocks without any ground point are left to `fill_nodata`, as before the plan. Binary `out-baseline/cassini-dem-step1-fallback`, outputs `out-dem-step1-fallback-<dataset>`. Against the baseline:

  | | cliff pixels | 202 objects | 205 objects |
  |---|---|---|---|
  | poussebout | 2 095 035 → 2 103 436 (+0.4%) | 2348 → 2460 (+4.8%) | 193 → 159 |
  | arselle | 3 216 357 → 3 328 893 (+3.5%) | 3219 → 3646 (+13.3%) | 107 → 83 |

  Per tile −1.5% to +9.0% cliff pixels. Contours: total length ±0.0%, lines −0.2% / +0.1%, per-level median change 0.02%; only poussebout raw 1680 m (14 → 16 m) changes by more than 5%. Crops: 0.25–0.96% of `contours.png` pixels differ, 2.2% of `cliffs.png` on the steep crops (scattered dots), ~0% on the others. Vegetation rasters still byte-identical. Timings back to the baseline: LiDAR stage 5.9–6.9 s per tile, fill 0.36–0.52 s, full batch 20.2 s (poussebout) / 19.7 s (arselle). `measure_dem_render`'s "with ground points" share now counts the fallback cells, so it equals the 1 m figures. The 205 boulders still drop (−18% / −22%), not investigated. Step 2's IGN DEM is already filled, so the fallback only applies to the DEM from the points.
- **Review fixes:** `MAX_TILE_SIDE` lowered from 10 km to 5 km (the DEM accumulators take 64 MB per km² over the header bounds, 4× the 1 m DEM). `VEGETATION_BLOCK_SIZE` is now an `f64` used everywhere (no more `1.0` literal in `vegetation.rs`). The fill log counts only the cells inside the tile.
- **Docs:** `what-and-why.mdx` (DEM cells are 0.5 m, vegetation 1 m, and the 1 m fallback) and the two cliff thresholds in `configuration-reference.mdx`, which said "one meter by one meter cell" although the slopes have always been 0.5 m.

### Step 2 (branch `dem-vegetation/step-2-ign-dem`, cut from `develop`)

- **IGN file** (WMS GetMap of the plan's template, 4 tiles each for poussebout and arselle in `in-poussebout-ign/` and `in-arselle-ign/`, LAZ symlinked): Float32, uncompressed strips, 2000×2000, nodata `-9999` (GDAL_NODATA tag ` -9999`), 100% valid cells, PixelIsArea, tiepoint at (min_x − 0.25, max_y + 0.25) as the plan says. **No vertical CRS key.** `ProjectedCSTypeGeoKey` is **32767 (user-defined)**: the GeoKeys spell out Lambert-93 (`ProjCoordTransGeoKey` 8 = LCC 2SP, parallels 49/44, false origin 3°/46.5°, 700000/6600000 in GeoDoubleParams, metres) with a citation `EPSG:2154|` and an unknown datum (gdalinfo: "unretrievable - using WGS84").
- **`Grid::read_geotiff`** already accepted any user-defined CRS. It is now stricter: a user-defined CRS with a projection must be LCC 2SP with Lambert-93's six parameters (false or natural origin keys) and metres; one without a projection, or no CRS key, is still assumed to be Lambert-93. New `GridExtent::read_geotiff` reads the georeferencing without decoding the cells (for `batch`'s matching).
- **Code:** [external_dem.rs](../../src/external_dem.rs) reads the DEM as `f32` (any square cell size; a DEM without nodata gets -9999), errors if it doesn't overlap the tile, warns if a side is uncovered by more than one of its cells. [lidar.rs](../../src/lidar.rs) reads it before cleaning the output directory, skips the ground accumulation (`TileRasters::ground_dem` is an `Option<GroundDem>`) and writes the DEM unchanged as `dem.tif`, Deflate, on its own grid. `batch` matches each `.tif`/`.tiff` (case-insensitive) of the input directory to the tile containing its extent's centre (`assign_dems` in [batch.rs](../../src/batch.rs)); not done with `--skip-lidar`. `render` and the mosaic are unchanged: a test in [buffer.rs](../../src/buffer.rs) checks the tie rule on half-cell-offset sources (last column and row from the east and south neighbours).
- **CLI change for mapant:** `cassini lidar <laz> [--dem <dem.tif>] [-o dir] [-C config]`: `--dem` is optional (it was required before the review, below). mapant's `lidar fix` can pass the downloaded IGN tile instead of replacing `dem.tif` afterwards (the 2-decimal quantization is no longer needed; `dem.tif` is the IGN tile, about 12.5 MB Deflate). `process` takes an optional `--dem`. `render` and `batch` options unchanged.
- **Acceptance:** full batches with `PATH=/nonexistent` on both datasets (binary `out-baseline/cassini-dem-step2-ign`, outputs `out-ign-<dataset>`, from copies of `out-dem-step1-fallback-<dataset>`): every `dem.tif` is cell-for-cell its IGN download (type, nodata, extent, values); vegetation rasters byte-identical to step 1. Mixed batch (`in-poussebout-mixed/`, IGN DEMs for 907000_6455000, renamed `renamed-dem.TIFF`, and 908000_6454000): one warning listing the two other tiles, their `dem.tif` identical to step 1's. `lidar` without `--dem` is a clap error; with another tile's DEM, "outside tile" error. `cargo test` green.
- **Fill:** inside the batch, the interior tile has no cell left to fill, edge tiles 2000 (the east or south strip), the south-east corner tile 3999. Fill 0.26–0.28 s per tile.
- **Timings:** LiDAR stage per tile with an IGN DEM 6.1–7.1 s (poussebout), 4.5–6.8 s (arselle), the same as step 1: decompression dominates. Full batch 20.4 s / 18.2 s. Single `lidar --dem` on one tile 4.2 s.
- **`dem_with_buffer.tif`, step 1 points DEM vs IGN, cells of the tile** (mean / p99 / max absolute difference, m):
  - poussebout 907000_6454000 0.21 / 1.45 / 43.1, 907000_6455000 0.17 / 1.26 / 95.8, 908000_6454000 0.19 / 0.95 / 25.6, 908000_6455000 0.13 / 0.72 / 7.3
  - arselle 926000_6448000 0.07 / 0.47 / 14.7, 926000_6449000 0.09 / 0.52 / 8.5, 927000_6448000 0.41 / 2.97 / 127.9, 927000_6449000 0.17 / 1.17 / 62.5
  - Small differences everywhere (the 0.25 m shift on slopes); the large ones on rock faces, where the points DEM jumps by up to 80 m between adjacent 0.5 m cells (ground points from the top and the foot of the face) while IGN's stays smooth (e.g. poussebout (907432.25, 6455089.25): 1280.3 vs 1376.0).
- **Cliffs drop, contours stay** (step 1 fallback → IGN, `measure_dem_render`):

  | | cliff pixels | 202 objects | 205 objects |
  |---|---|---|---|
  | poussebout | 2 103 436 → 1 076 925 (−48.8%) | 2460 → 1183 (−51.9%) | 159 → 83 |
  | arselle | 3 328 893 → 2 391 126 (−28.2%) | 3646 → 2480 (−32.0%) | 83 → 24 |

  Per tile −18% to −78% cliff pixels (poussebout's east tiles −76/−78%). Contours: total length −0.4% / −0.3% raw, −0.1% / −0.0% smoothed; lines −1.3% / −2.2% raw. Levels over 5%: poussebout raw 1680 m (16 → 14 m), smoothed 1645 m (+7.4%); arselle 712.5 m (4 → 8 m), 807.5–815 m (+6–8%). Crops (same areas as step 1): the main cliff bands stay, the isolated dots and noisy fringes around them go; `cliffs.png` differs on 13.1% / 6.2% of the steep crops' pixels, ~0% elsewhere; `contours.png` 2.3–7.5%. The cliff thresholds were tuned on the points DEM; the user retunes them after step 3.
- **Docs:** "Use IGN DEMs" in the batch guide (matching rule, download URL template, 0.25 m shift, fewer cliffs), `--dem` in the single-tile guide and the CLI reference (`process`, `lidar`, `batch`), one paragraph in `what-and-why.mdx`.
- **Review fixes:**
  - **Crop:** an external DEM is cropped to the cells that intersect its tile (`Grid::window`). The mosaic pastes the neighbours over the tile's own DEM, so two overlapping DEMs made each tile take the other's near their shared edge: seams. Cropped, they overlap at most on the cells straddling the edge; IGN tiles are unchanged (every cell intersects the tile). A larger DEM given to `process` no longer fills the buffer, nor is copied whole to `dem.tif`.
  - **Warning** when the DEM's cells are larger than `DEM_BLOCK_SIZE` (nearest neighbour gives stepped contours).
  - **`lidar --dem` is optional**, as for `process`: the `lidar` step alone works again outside France.
  - **`batch` discovery:** `find_dems` returns a map from tile to DEM, used by `get_tiles_with_neighbors` (new `use_dem_files` argument) to build each tile with its DEM. Hidden files (macOS `._*`) are skipped, and a `.tif` that isn't a Lambert-93 DEM is ignored with a warning instead of stopping the batch. `--skip-lidar` warns when the input directory holds GeoTIFF files.
  - **`TileRasters`** holds a `TileDem` enum (`External(Grid)` / `Ground(GroundDem)`) instead of a `bool` and two `Option`s, so `dem.tif` is always written.
  - **CRS check:** a user-defined CRS whose `ProjectionGeoKey` (3074) names an EPSG projection must name Lambert-93's (18085).
  - **Tests:** the `external_dem` and `grid` temp files use `tempfile`.

### Step 3 (branch `dem-vegetation/step-3-vegetation-height`, cut from `develop`)

- **Code:**
  - [point_reader.rs](../../src/point_reader.rs) opens LAS/LAZ files: LAZ through `laz::ParLasZipDecompressor::selective` (X, Y, Z, classification; `laz` 0.13 is now a direct dependency), `.las` through `las::Reader`. It fills a `las::PointData` (`resize_for`), so the records are parsed by `las`'s own accessors rather than by hand: same values, same order. It asks the decompressor for at most the points left, as `las` does.
  - [lidar.rs](../../src/lidar.rs): `generate_dem_tiff_image_from_laz_file` (DEM pass; also the vegetation in `lidar-class` mode, the previous single pass), `generate_vegetation_density_tiff_images_from_laz_file` (height pass over a filled buffered DEM) and `generate_dem_and_vegetation_density_tiff_images_from_laz_file` (single tile: both, with the tile's DEM pasted onto its buffered grid without neighbours by `buffer::paste_with_buffer` and filled, in memory). `extent.txt` is written after the vegetation in both modes. In `height` mode with a DEM file the only pass is the vegetation one. `TileCells` now holds an `f64` origin, so `GroundElevations` bins points into the buffered DEM with the same `floor` rule (exact: `x - min_x` is exact by Sterbenz, so the buffered cell is the tile cell shifted by 400; a test checks it on a 1 cm grid). Height bands are `VEGETATION_LAYER_HEIGHTS = [0, 1, 4, 30]`, `(lower, upper]`.
  - [dem.rs](../../src/dem.rs): `create_dem_with_buffer` (mosaic, fill, write), `fill_dem_with_buffer`, `read_dem_with_buffer`, `create_contours_and_slopes`. [render.rs](../../src/render.rs): `render_tile_map` takes `DemWithBuffer::Mosaic` (`process`, `render`) or `Existing` (batch). `dem_with_buffer.tif` left `RENDER_STEP_FILES`.
  - [batch.rs](../../src/batch.rs): stages DEM → (barrier) filled DEM + height vegetation → render, through a shared `run_stage` helper. `--skip-lidar` checks every tile's `dem_with_buffer.tif` before cleaning anything, and errors listing the missing ones.
  - The vegetation pass logs the mean points per cell of each layer.
- **CLI change for mapant:** `cassini lidar <laz> [--dem <dem.tif>] [--vegetation-layers height|lidar-class] …`, default `height`. `lidar` alone has no neighbours, so its heights are above the tile's own DEM filled without them. On 907000_6454000 its height-mode rasters differ from the batch's on very few cells (mean absolute difference ≤ 6.3e-5 points per cell from the points DEM, ≤ 1.3e-4 with IGN's). Pass `--vegetation-layers lidar-class` to keep the previous vegetation. `render` unchanged. `process` and `batch` take the same option.
- **Outputs:** binary `out-baseline/cassini-dem-step3`, batches `out-s3-<lidar-class|height>-[ign-]<dataset>` (full, `--ocad`, `PATH=/nonexistent`, from copies of `out-dem-step1-fallback-<dataset>` / `out-ign-<dataset>`), logs next to them. `out-s3full-*.log` are the same batches with a build decoding every field (outputs deleted: byte-identical to the selective ones).
- **Acceptance:**
  - `lidar-class` against step 2, both DEM sources, both datasets: vegetation rasters, `dem.tif`, `dem_with_buffer.tif`, 2 m DEM, slopes, contours, form lines, every tile PNG and `merged-map.png` byte-identical (100 files per batch). `merged-map.ocd` differs, but **the step 2 binary also writes a different `merged-map.ocd` on every run** (same size, bytes reordered; two `--skip-lidar --ocad` runs of `cassini-dem-step2-ign`). Pre-existing, not investigated; `ocad_merge.rs` uses `HashMap`s.
  - `height` against `lidar-class`: `dem.tif`, `dem_with_buffer.tif`, 2 m DEM, slopes, contours (raw and smoothed), `cliffs.png`, `contours.png` byte-identical, all 4 batches.
  - Selective against full decoding: vegetation and terrain files byte-identical in all 8 batches.
  - `lidar` single tile: `lidar-class` output byte-identical to the batch's; no `dem_with_buffer.tif` written. `--skip-lidar` on a height batch: PNGs and merged map identical.
- **Timings** (per tile, 4 tiles, 3 threads, 12 cores; full decode → selective; LiDAR passes only, the filled DEM between them is 0.5–0.8 s):

  | | poussebout | arselle |
  |---|---|---|
  | points, `lidar-class` | 6.2–7.3 → 4.8–5.6 s | 6.0–8.7 → 4.4–6.2 s |
  | points, `height` (two passes) | 13.2–15.9 → 8.2–9.4 s | 10.4–12.9 → 7.6–11.6 s |
  | IGN, `lidar-class` | 6.8–8.0 → 5.8–6.9 s | 6.3–8.9 → 4.6–5.8 s |
  | IGN, `height` (DEM copy 0.8–1.0 s + one pass) | 7.3–8.3 → 5.5–6.2 s | 6.1–8.8 → 4.3–7.0 s |

  Full batches (selective): poussebout 18.2 / 26.8 / 20.1 / 20.0 s, arselle 19.2 / 28.3 / 19.1 / 21.8 s (same row order). Step 2 was 20.4 s (poussebout, IGN) / 18.2 s (arselle, IGN); step 1 fallback 20.2 / 19.7 s. Single `lidar` on 907000_6454000: `lidar-class` 4.2 s, `height` 6.9 s (2.8 + fill 0.5 + 3.6), `height --dem` 4.9 s.
- **Mean points per 1 m cell** (low / medium / high), `lidar-class` (same for both DEM sources) → `height` points DEM → `height` IGN DEM:
  - poussebout 907000_6454000: 1.01 / 0.68 / 12.72 → 1.03 / 3.09 / 10.03 → 1.08 / 3.06 / 10.08
  - poussebout 907000_6455000: 0.60 / 0.34 / 10.92 → 0.66 / 1.24 / 9.86 → 0.67 / 1.24 / 9.85
  - poussebout 908000_6454000: 0.43 / 0.21 / 16.37 → 0.43 / 0.80 / 15.50 → 0.48 / 0.79 / 15.53
  - poussebout 908000_6455000: 0.56 / 0.35 / 14.45 → 0.65 / 1.15 / 13.48 → 0.65 / 1.15 / 13.49
  - arselle 926000_6448000: 0.48 / 0.25 / 6.97 → 0.55 / 0.99 / 6.13 → 0.54 / 1.00 / 6.12
  - arselle 926000_6449000: 0.56 / 0.33 / 5.39 → 0.67 / 1.25 / 4.37 → 0.64 / 1.25 / 4.38
  - arselle 927000_6448000: 1.18 / 0.50 / 12.05 → 1.03 / 1.45 / 10.73 → 1.12 / 1.55 / 10.83
  - arselle 927000_6449000: 0.78 / 0.64 / 9.82 → 0.92 / 2.49 / 7.71 → 0.86 / 2.51 / 7.75

  Medium vegetation is 2–5× denser by height (LiDAR HD classes much of the 1–4 m vegetation as high), high 5–20% sparser, low about the same. The DEM source barely matters. PDAL's estimate for 907000_6454000 was 1.2 / 2.8 / 10.4.
- **Crops** (step 1 areas, `lidar-class` | `height` | diff, `vegetation.png` and `full-map.png`): `full-map.png` differs on 1.6–1.9% of the flat crops, 2.7–12.5% of the steep ones (mostly hidden under cliffs), 12.7% (poussebout) and 54% (arselle) of the corner crops; the maps are much greener in `height` mode (medium green). No seam at tile corners. The thresholds are unchanged; the user retunes them from the densities above.
- **Release:** `CHANGELOG.md` created (entry 0.19.0 for the three steps), version 0.19.0, `RELEASING.md` points to the changelog. Docs: `--vegetation-layers` in the CLI reference (and `--skip-lidar`'s reuse of `dem_with_buffer.tif`, its error, and rerunning after adding or removing tiles), "Vegetation layers" sections in the single-tile and batch guides, `what-and-why.mdx` (selective decompression, two passes in `height` mode).
