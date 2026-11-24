# Plan: replace PDAL, GDAL and PROJ with Rust code

Date: 2026-11-24

## Goal

Cassini should ship as a single self-contained binary for Linux, macOS and Windows, with no Docker or Conda. Today it shells out to three external toolsets:

| Call | Where | Replaced in |
|---|---|---|
| `pdal pipeline` (ground DEM, height above ground, voxel thinning, vegetation counts) | [lidar.rs](../../src/lidar.rs) | Step 4 |
| `gdalbuildvrt` + `gdal_translate -projwin` (stitch neighbour tiles, crop with buffer) | [buffer.rs](../../src/buffer.rs) | Step 2 |
| `gdal_translate -tr 2 -r average` (2 m DEM) | [dem.rs](../../src/dem.rs) | Step 2 |
| `gdaldem slope` | [dem.rs](../../src/dem.rs) | Step 2 |
| `cs2cs` from PROJ (Lambert‑93 → WGS84 for the Overpass bbox) | [download.rs](../../src/download.rs) | Step 2 |
| `gdal_fillnodata` (fill DEM holes) | [dem.rs](../../src/dem.rs) | Step 3 |
| `gdal_contour -i 2.5` (raw contours shapefile) | [dem.rs](../../src/dem.rs) | Step 3 |
| `ogr2ogr` (OSM XML → `multipolygons.shp` and `lines.shp` in EPSG:2154) | [render.rs](../../src/render.rs) | Step 5 |

We are replacing these with plain Rust rather than embedding GDAL through the `gdal` crate. Cassini uses only a small part of GDAL, almost entirely on rasters that share an aligned integer grid. A bundled GDAL build would need a C/C++ toolchain on every CI target and would have to ship PROJ's `proj.db` next to the binary. Call sites would also need rewriting against the C API, since `gdal_fillnodata` is a Python script.

## How to use this plan

- **One step per Claude Code session.** Start a session with: *"Implement step N of `specs/plans/2026-11-24-pdal-gdal-rust-rewrite.md`."*
- **Branches:** each step works on its own branch from `develop` (`rust-pipeline/step-N-<slug>`) and is merged before any step that depends on it.
- **Order:** steps 1 → 2 are sequential. After step 2, steps 3, 4 and 5 touch different files and can be done in any order, or in parallel. Step 6 needs 3, 4 and 5. Step 7 needs 6.
- **At the end of each session:**
  - Update the status table below.
  - Append any finding the next steps need to "Notes from completed steps" at the bottom: semantic surprises, tolerances chosen, parity numbers.
  - Keep notes short. This file is the hand-over between sessions.
- **Builds:** always use `cargo build --locked` / `cargo install --path . --locked`. Without `--locked`, `cargo install` ignores `Cargo.lock` and re-resolves dependencies.

## Status

| Step | Title | Depends on | Status |
|---|---|---|---|
| 1 | Foundations: GeoTIFF I/O, comparison tool, reference outputs | – | Done |
| 2 | Simple raster ops and coordinate conversion | 1 | Done |
| 3 | DEM hole filling and contour extraction | 1, 2 | Done |
| 4 | LiDAR pipeline (PDAL replacement) | 1 | Not started |
| 5 | OSM import (ogr2ogr replacement) | 2 | Not started |
| 6 | Cleanup, docs and end-to-end verification without external tools | 3, 4, 5 | Not started |
| 7 | Packaging and release builds for the three platforms | 6 | Not started |

## Shared context

### Test data on the dev machine

- **Inputs:** `in-poussebout/` holds four LiDAR HD COPC tiles (`LHD_FXX_0907_6455…`, `0907_6456`, `0908_6455`, `0908_6456`). Other datasets: `in-arselle`, `in-correncon`, `in-evg`, `in-tunnels`.
- **Existing outputs:** `out-poussebout/` is a 2×2 tile grid of outputs from the current toolchain. That includes PDAL outputs (`dem.tif`, `*_vegetation.tif`), `.osm` files and render intermediates. Both `in*` and `out*` are git-ignored.
- **PDAL is not installed** on the dev machine, so the LiDAR stage cannot be re-run with PDAL; the existing `out-*` PDAL outputs are the only reference. To exercise the render stages, copy an `out-*` directory to the scratchpad and run `cassini batch -o <copy> --skip-lidar in-poussebout` (about 8 s, add `--ocad` to cover the OCAD merge).
- **GDAL CLI tools and `cs2cs` are installed** (`gdalinfo`, `ogrinfo`, `gdal_contour`, …), so GDAL reference outputs can be regenerated with the pre-rewrite binary. Python has no `numpy` or `osgeo`, so `gdalcompare.py` is unavailable: use the Rust comparison tool from step 1.

### Formats of the current intermediate files

Checked with `gdalinfo` on `out-poussebout/907000_6454000_908000_6455000`:

| File | Size | Pixel | Type | NoData | Compression |
|---|---|---|---|---|---|
| `dem.tif` | 2000×2000 | 1 m | Float64 | -9999 | Deflate, predictor 3 |
| `low/medium/high_vegetation.tif` | 1000×1000 | 1 m | Byte | 255 | Deflate, predictor 2 |
| `dem_with_buffer.tif` | 2800×2800 | 0.5 m | Float64* | -9999 | none |
| `dem_2m_with_buffer.tif` | 700×700 | 2 m | Float64* | -9999 | none |
| `slopes.tif` | 2800×2800 | 0.5 m | Float32 | -9999 | none |
| `contours-raw/contours-raw.shp` | 945 features | – | LineString, fields `ID` (Integer), `elev` (Real) | – | – |

\* These outputs may predate the switch to `Float32` in [buffer.rs](../../src/buffer.rs) / [dem.rs](../../src/dem.rs). Read the current code, not these files, for the expected type.

### Known DEM extent bug

In [lidar.rs](../../src/lidar.rs), the DEM `writers.gdal` stage has `"resolution": 1` but `width/height = (max - min) * 2`. As a result, `dem.tif` covers 2 km × 2 km from the tile's lower-left corner, and only the first quarter (the real tile) contains data. That is why `STATISTICS_VALID_PERCENT` is 20.84%, which equals 83.4% of the tile.

Nothing visible breaks, because the overhang is nodata and the buffer step crops it. The Rust rewrite (step 4) should write a **1 m DEM covering exactly the tile**. That should leave `dem_with_buffer.tif` unchanged.

Do **not** switch to a true 0.5 m DEM as part of this plan. On the poussebout tile there are 6.24 ground points/m²: 83.4% of 1 m cells contain at least one ground point, but only 59.7% of 0.5 m cells do. A 0.5 m mean grid would leave about 40% of cells for `fillnodata` to make up and would be noisier. Improving DEM quality (bilinear upsampling instead of nearest, radius-based gridding) is a separate experiment after this plan.

### Writing GeoTIFFs with the `tiff` crate

The `tiff` crate has no GeoTIFF API, but it writes arbitrary tags through `image.encoder().write_tag(...)`. This was verified with tiff 0.9.1: GDAL reads the result with the correct CRS, origin, pixel size and nodata. The tags:

- `ModelPixelScaleTag` (33550): `[res, res, 0.0]`
- `ModelTiepointTag` (33922): `[0, 0, 0, min_x, max_y, 0]`
- `GeoKeyDirectoryTag` (34735): `[1,1,0,3, 1024,0,1,1, 1025,0,1,1, 3072,0,1,2154]`, which says projected, pixel-is-area, EPSG:2154
- `GdalNodata` (42113): ASCII string, e.g. `"-9999"`

tiff 0.11 supports the horizontal predictor (2) but **not** the floating-point predictor (3). Float rasters are therefore written without a predictor, which makes them somewhat larger and is acceptable.

### Parity methodology

Every replacement is checked against the output of the tool it replaces, on all four poussebout tiles:

1. **Rasters:** use the comparison tool from step 1. Report extent and size equality, the count of cells where only one side has nodata, and max / mean / p99 absolute difference over cells valid on both sides.
2. **Vectors:**
   - Feature count per layer.
   - For lines: total length.
   - For polygons: total area, per relevant tag (`natural`, `landuse`, `building`, `highway`, `waterway`, …).
   - A spot-check of a few geometries.
3. **Final render:** run the full `--skip-lidar` batch (or the full batch, for step 4) before and after, and compare the final images pixel by pixel. List differing pixels as a percentage and look at a diff image.

Each step section states its acceptance threshold. "Exact" means bit-identical values. Where exactness is not achievable (different interpolation details, point order), the step records the measured differences under "Notes from completed steps".

---

## Step 1 — Foundations: GeoTIFF I/O, comparison tool, reference outputs

**Goal:** the infrastructure every later step uses.

**Tasks**

1. **Upgrade `tiff` to 0.11.x** and adapt the existing decoders in [raster.rs](../../src/raster.rs), [vegetation.rs](../../src/vegetation.rs) and [pullautin_smooth_contours.rs](../../src/pullautin_smooth_contours.rs). `cargo test` must pass, and a `--skip-lidar` batch on a copy of `out-poussebout` must produce identical final images.
2. **Add a georeferenced grid type** in a new `src/grid.rs` (or extend `raster.rs` if that reads better). For example, `Grid<T>`: `min_x`, `max_y`, `resolution`, `width`, `height`, `nodata`, row-major `Vec<T>` (row 0 at the top). It needs:
   - `read_geotiff(path)` for `f32`, `f64` and `u8`. Read the GeoTIFF tags and `GdalNodata`, and convert other numeric types if needed.
   - `write_geotiff(path, compression)`, writing the tags listed in "Writing GeoTIFFs with the `tiff` crate".
   - Helpers to map between pixel and world coordinates (pixel centres at `min_x + (col + 0.5) * res`).
   - Unit tests: a read/write round trip, plus one test that writes a file and checks `gdalinfo` output when `gdalinfo` is on `PATH` (skip otherwise).
3. **Add a raster comparison tool** as `examples/compare_rasters.rs`. Run it with `cargo run --release --example compare_rasters -- a.tif b.tif`. It prints the metrics from "Parity methodology", optionally `--crop <min_x,min_y,max_x,max_y>`, and writes a diff PNG with `--diff-png`.
4. **Add a final-image comparison**, either in the same example or a sibling one: compares two PNG/WebP renders, prints the share of differing pixels, and writes a diff image.
5. **Snapshot the reference outputs:**
   - Build the pre-rewrite binary (current `develop`).
   - Copy `out-poussebout` to `out-reference-poussebout` (git-ignored) and run `cassini batch -o out-reference-poussebout --skip-lidar --ocad in-poussebout` on it, so all GDAL intermediates exist.
   - Keep a copy of the pre-rewrite binary as `out-reference-poussebout/cassini-reference` to regenerate references later.
   - Write the exact commands to the notes section.

**Acceptance:**
- `cargo test` is green.
- A grid written by `write_geotiff` opens in `gdalinfo` with EPSG:2154, the right origin, pixel size and nodata.
- The tiff upgrade leaves final renders identical.

**Out of scope:** replacing any external tool.

---

## Step 2 — Simple raster ops and coordinate conversion

**Goal:** remove `gdalbuildvrt`, both `gdal_translate` uses, `gdaldem slope` and `cs2cs`. These are all small and have well-defined semantics, so parity should be exact or near-exact.

**Tasks**

1. **Buffer mosaic and crop** in [buffer.rs](../../src/buffer.rs). Replace `gdalbuildvrt` + `gdal_translate -projwin … -tr … [-ot …]`:
   - Allocate the output grid for the buffered extent at the requested resolution, filled with nodata.
   - For each source tile (the tile itself, then its neighbours), sample with nearest neighbour at output pixel centres. A source nodata value must not overwrite valid data. This reproduces the VRT's transparent compositing.
   - The DEM is 1 m and resampled to 0.5 m here. Make sure the nearest-neighbour rounding matches GDAL at pixel edges, so the result is exact.
   - Keep writing `*_with_buffer.tif` files. Keeping things in memory is a later optimisation, outside this plan.
2. **2 m DEM** in [dem.rs](../../src/dem.rs) (`gdal_translate -tr 2 -r average`):
   - Average of the 4×4 block of 0.5 m cells, ignoring nodata.
   - The output is nodata only if all 16 cells are nodata.
   - Check GDAL's exact behaviour (nodata handling and weighting at block edges) on the reference before deciding.
3. **Slope** (`gdaldem slope`):
   - Horn's method in degrees, scale 1.
   - GDAL's default edge behaviour: without `-compute_edges`, the border pixels and pixels with a nodata neighbour are nodata.
   - Output Float32, nodata -9999.
4. **Coordinate conversion** in [download.rs](../../src/download.rs):
   - Replace `cs2cs` with a small `src/projection.rs` providing `lambert93_to_wgs84(x, y)` and `wgs84_to_lambert93(lon, lat)`. The forward direction is needed in step 5.
   - Either use the `proj4rs` crate or implement the Lambert conformal conic formulas directly: GRS80, standard parallels 44°/49°, origin 46.5°N 3°E, false easting/northing 700000/6600000.
   - Treat RGF93 and WGS84 as the same datum, as `cs2cs` does with `+init=epsg:…`.
   - Unit-test against `cs2cs` values for a few points spread across France, to within 1e-8°.
5. Remove the replaced `Command::new` calls. Keep `process.rs` while other external calls remain.

**Acceptance:**
- `compare_rasters` reports exact equality, or a max difference ≤ 1e-6 for floats, against `out-reference-poussebout` for `*_with_buffer.tif`, `dem_2m_with_buffer.tif` and `slopes.tif` on all four tiles.
- Final renders are identical, or any differing pixel is explained in the notes.

---

## Step 3 — DEM hole filling and contour extraction

**Goal:** remove `gdal_fillnodata` and `gdal_contour`.

**Tasks**

1. **Fill nodata** (`gdal_fillnodata` with default options: max search distance 100 pixels, 0 smoothing iterations):
   - Port the algorithm from GDAL's `alg/rasterfill.cpp` (MIT licence; keep the attribution comment): an inverse-distance-weighted average of the nearest valid pixels found in the scan directions, up to the max distance.
   - Read the source rather than guessing the search pattern.
   - According to [the render performance report](../research/render-pipeline-performance-report.md), DEM buffering and hole filling take about 49% of render time on a representative tile. Measure the Rust version against the GDAL one and record both.
2. **Contours** (`gdal_contour -a elev -i 2.5`):
   - Use the `contour` crate (`ContourBuilder::lines`, already a dependency) with smoothing off.
   - Set the origin and step so vertices are on pixel centres, as in GDAL.
   - Levels at multiples of 2.5 m.
   - Write `contours-raw/contours-raw.shp` with the same fields as today (`ID` Integer, `elev` Real) and an EPSG:2154 `.prj`, because [pullautin_smooth_contours.rs](../../src/pullautin_smooth_contours.rs) and [pullautin_contours_render.rs](../../src/pullautin_contours_render.rs) read it.
   - Check how those readers use the fields and line direction. GDAL keeps higher ground on a consistent side; if the readers depend on that, match it.
   - Handle nodata: GDAL uses the band's nodata value. After filling, only holes larger than the search distance remain.

**Acceptance:**
- **Fill:** on all four tiles, the nodata masks are identical and the p99 absolute difference is ≤ 0.01 m, with any larger maximum explained. Speed is not worse than GDAL.
- **Contours:**
  - Total contour length per level within 0.5% of `gdal_contour`.
  - Feature count may differ if segments are joined differently, so record it.
  - Final renders visually identical, with differing pixels quantified in the notes.

---

## Step 4 — LiDAR pipeline (PDAL replacement)

**Goal:** remove PDAL. Implement the pipeline in [lidar.rs](../../src/lidar.rs) with the `las` crate.

**Tasks**

1. **Upgrade `las`** from 0.8 to the latest 0.x. Enable parallel LAZ decompression; the feature is named `laz-parallel` in recent versions, so check. The header read in `lidar.rs` must keep working. The inputs are COPC files (`.copc.laz`), which are valid LAZ 1.4 and can be read sequentially.
2. **Implement it as two streaming passes**, never holding all points in memory (tiles have about 35 M points):
   - **Pass 1 — DEM:** for ground points (class 2), compute the mean Z per 1 m cell, over a grid covering **exactly the tile** (this fixes the extent bug).
     - Binning follows PDAL `writers.gdal` `binmode: true`: a point falls into the cell containing it. Check how PDAL handles points exactly on the max edge.
     - Write `dem.tif`: Float64 or Float32 (use whatever the buffer step expects), nodata -9999, Deflate.
   - **Pass 2 — vegetation:** for each point:
     - Compute height above ground as `Z − DEM(cell containing the point)`, following `filters.hag_dem`.
     - Check in PDAL's source (`filters/HagDemFilter.cpp`) what happens when the DEM cell is nodata, and match it.
     - Apply `filters.voxeldownsize` with `cell: 0.5`, `mode: first`: keep only the first point seen in each 0.5 m voxel, using a hash set of voxel keys. Check in PDAL's source how the voxel grid origin is defined.
     - Classify non-ground points with height above ground in (0, 1] as low, (1, 4] as medium and (4, 30] as high.
     - Count points per 1 m cell into the three grids.
   - Write `low/medium/high_vegetation.tif` matching the reference: Byte, nodata 255, Deflate with horizontal predictor.
     - Check what PDAL writes for cells with zero points (0 or nodata 255), and how [vegetation.rs](../../src/vegetation.rs) interprets it.
     - Counts cannot exceed 255 after 0.5 m voxel thinning within these height bands, but saturate anyway.
3. **Keep the output contract:**
   - The output directory cleanup that keeps the `.osm` file.
   - `ensure_file` checks.
   - `extent.txt` written last as the completion marker.
4. **Multithreading:** tiles are already processed in parallel by `batch`. Check how many threads the decompressor should use so the two levels of parallelism don't oversubscribe the CPU.

**Acceptance:** PDAL is not available, so compare against the existing `out-poussebout` outputs (in `out-reference-poussebout`):
- **`dem.tif`:** cropped to the tile, exact or float-rounding-level differences (max ≤ 1e-6 m), with an identical nodata mask.
- **Vegetation rasters:** PDAL reads COPC files in octree order, so the "first" point per voxel may differ and exact parity is not expected. Report per-raster mean count difference and the share of cells that differ, and judge them on the final render: vegetation layers should be visually indistinguishable.
- Run a full batch (not `--skip-lidar`) on `in-poussebout` and record the LiDAR stage time per tile in the notes.

---

## Step 5 — OSM import (ogr2ogr replacement)

**Goal:** remove `ogr2ogr`. Convert the `.osm` file produced by [download.rs](../../src/download.rs) into the two shapefiles that [vectors.rs](../../src/vectors.rs) reads.

**Tasks**

1. **Find exactly what the consumers need.**
   - [vectors.rs](../../src/vectors.rs) reads `shapes/multipolygons.shp` and `shapes/lines.shp`, and uses these fields: `natural`, `building`, `landuse`, `highway`, `waterway`, `itermittent` (sic), `seasonal`, `railway`, `aerialway`, `tunnel`, plus `other_tags`. `other_tags` is parsed for `power`, `natural` and `tunnel`, in OGR's `"key"=>"value",…` hstore format.
   - Check [ocad_export.rs](../../src/ocad_export.rs) and any other readers too.
   - Only these fields need reproducing, but keep the OGR column names and `other_tags` format so `vectors.rs` is untouched.
2. **Parse the OSM XML** with `quick-xml`: nodes, ways and relations with tags.
3. **Build the `lines` layer.** Reproduce OGR's OSM driver rules from its default `osmconf.ini`: which ways become lines and which closed ways become areas instead (the `closed_ways_are_polygons` tag list). Ways that are members of multipolygon relations are handled as in OGR.
4. **Build the `multipolygons` layer:**
   - Closed ways that qualify as areas.
   - `type=multipolygon` relations: join member ways into closed rings, assign outer/inner roles, and nest holes in their outer ring.
   - Skip broken relations with a warning, as OGR does.
5. **Reproject** WGS84 → Lambert‑93 with `projection.rs` from step 2. Write shapefiles with the `shapefile` crate (Polygon for multipolygons, PolyLine for lines) and a `.prj`.
6. Keep `ogr2ogr`'s `-nlt MULTIPOLYGON` / `LINESTRING` geometry types as the `shapefile` crate represents them.

**Acceptance:** on all four poussebout tiles plus `in-tunnels`, which exercises the tunnel tags:
- Feature counts per layer and per main tag within ±1%.
- Total area per tag for polygons and total length per tag for lines within 0.5%.
- Any mismatch is explained in the notes.
- Final renders visually identical, with differing pixels quantified.

---

## Step 6 — Cleanup, docs and end-to-end verification without external tools

**Goal:** prove Cassini runs with no external tools, and remove what is left of the old setup.

**Tasks**

1. **Remove dead code:**
   - Remove the external-command helpers in [process.rs](../../src/process.rs) and the related `CassiniError` variants in [error.rs](../../src/error.rs) if nothing uses them any more. Keep the error types other code still needs.
   - Update the CLI `about` text in [main.rs](../../src/main.rs) that says GDAL and PDAL must be installed.
2. **End-to-end check without external tools:** run full batches (LiDAR included) on every `in-*` dataset with `env PATH=/nonexistent ./target/release/cassini …`, so any leftover external call fails loudly.
   - Compare final renders against references: build references for the other datasets with `cassini-reference` and `--skip-lidar`, as in step 1.
   - Record total time per dataset compared with the reference binary.
3. **Grep check:** `Command::new` must only remain in tests, if anywhere.
4. **Docs:**
   - Rewrite [installation-and-setup.mdx](../../docs/src/content/docs/guides/installation-and-setup.mdx): download a binary, no Docker or Conda.
   - Update [index.mdx](../../docs/src/content/docs/index.mdx), [what-and-why.mdx](../../docs/src/content/docs/what-and-why.mdx), [cli-reference.mdx](../../docs/src/content/docs/reference/cli-reference.mdx) and the README.
   - Delete or reword the glossary entries that become irrelevant (`pdal.md`, `gdal.md`, `miniconda.md`, `docker.md`).
5. **Dockerfile:** stop using the `pdal/pdal` base image. Either use a slim base image with only the binary, or drop the image if it is no longer wanted. Ask the user which.

**Acceptance:**
- All datasets run with an empty `PATH`.
- Renders match the references within the tolerances accepted in steps 2–5.
- `cargo test` is green.
- The docs no longer mention installing PDAL, GDAL, Docker or Conda.

---

## Step 7 — Packaging and release builds for the three platforms

**Goal:** downloadable binaries that work on a fresh Linux, macOS and Windows machine.

**Tasks**

1. **Dependency modernisation** that affects cross-platform builds:
   - Upgrade `reqwest` from `=0.10.10` with `native-tls-vendored` to the current version with `rustls`. This removes vendored OpenSSL, which is the hardest part to build on Windows and macOS. Port the blocking calls in [download.rs](../../src/download.rs).
   - Bump `skia-safe` only if the current version has no prebuilt binaries for a target. It is a large API jump (0.75 → 0.15x), so otherwise leave it.
2. **Release workflows:**
   - The repo already has `.github/workflows/{linux,macos-amd64,macos-arm64,windows}-release.yml`. Either update them or replace them with one `dist` (cargo-dist) setup.
   - Recommend one, and ask the user before switching.
   - Targets: `x86_64-unknown-linux-gnu`, built on the oldest supported Ubuntu runner for glibc compatibility; `aarch64-apple-darwin`; `x86_64-apple-darwin`; `x86_64-pc-windows-msvc`.
   - Add `--locked`.
   - Remove the Docker Hub publish step if step 6 dropped the image.
3. **Smoke test in CI on each OS:** run `cassini lidar` + `render` on one small LAZ tile with `PATH` cleared, and check the output files exist. The LAZ fixture must be small: crop one tile to roughly 100 × 100 m with the new code, or download it at test time.
4. **Installers:** shell and PowerShell install scripts (with `dist`, these come for free) and a Homebrew tap if wanted.
5. **Signing:** document, but do not implement without the user's accounts:
   - macOS notarization needs an Apple Developer account.
   - Windows code signing avoids SmartScreen warnings.
   - Install scripts avoid macOS quarantine for command-line use.

**Acceptance:**
- A tagged pre-release produces artifacts for all four targets.
- The CI smoke test passes on Linux, macOS and Windows.
- A binary downloaded on a machine without GDAL or PDAL renders a tile.

---

## Notes from completed steps

_Each session appends findings here: decisions, tolerances, measured parity, timings, surprises._

### Step 1 (branch `rust-pipeline/step-1-foundations`, cut from `remove-pdal-gdal` = `develop` + this plan)

- **tiff 0.11.3:** the existing decoders compiled unchanged. The lockfile also bumps `weezl` 0.1.8 → 0.1.12 and adds `fax`, `zune-jpeg`. `image` 0.25 still pulls `tiff` 0.9.1 alongside; that is harmless. All 17 final PNGs (`*/full-map.png`, `*/vegetation.png`, `*/contours.png`, `*/cliffs.png`, `merged-map.png`) and all GDAL intermediates are byte-identical to the reference.
- **`merged-map.ocd` is not deterministic:** two runs of the reference binary give different bytes (same size). Don't use it for parity; compare the PNGs.
- **`src/grid.rs`:** `Grid<T>` for `f32`, `f64`, `u8` (trait `Sample`).
  - `read_geotiff` converts any integer or float band to `f32`/`f64` with `as`, like `gdal_translate -ot` (`u8` reads only `u8`). The nodata tag is parsed as the stored type and converted the same way, so nodata cells still match it. `read_geotiff_with_type` also returns the stored `SampleType`.
  - It honours PixelIsPoint like GDAL and reads GDAL's floating-point predictor (checked on `dem.tif`). It rejects a geographic CRS or an EPSG code other than 2154, and accepts a user-defined CRS or none.
  - `write_geotiff(path, Compression::{None, Deflate})` adds the horizontal predictor for `u8` only.
  - `cell_at(x, y)` puts points on a cell edge in the east/south cell, so the east and south grid edges return `None`, as does a NaN coordinate. `index`/`get`/`set` `debug_assert!` that the cell is inside the grid.
  - It replaces `raster.rs` (`F32Raster`, removed) and the TIFF decoders of `vegetation.rs` and `pullautin_smooth_contours.rs`: every TIFF read goes through `Grid::read_geotiff`. The local `Grid` structs of `cliff_features.rs` and `vegetation_areas.rs` are now `Window`.
- **Library target:** `src/lib.rs` exports `error` and `grid`; `main.rs` imports them with `use cassini::{error, grid};`, so `crate::grid` still works in the binary's modules. Modules the examples need go in `lib.rs`, and may only depend on other `lib.rs` modules.
- **Comparison tools** (exit 0 = equal, 1 = different, 2 = error):
  - `cargo run --release --example compare_rasters -- a.tif b.tif [--crop min_x,min_y,max_x,max_y] [--diff-png d.png]`. Both rasters must have the same resolution and aligned cells (else exit 2); the window is `a`'s cells inside `b`'s extent and the crop. A different stored type or nodata value is a difference, and so is a different extent unless `--crop` is given. For `dem.tif` vs a tile-sized DEM, pass the tile as `--crop`. The reference `dem.tif` tile 907000_6454000 has 166 290 nodata cells out of 1 000 000.
  - `cargo run --release --example compare_images -- a b [--diff path] [--tolerance n]`. `a` and `b` are two images, or two output directories, in which case the PNG/WebP images of both are matched by relative path (an image missing on either side is a difference) and `--diff` is a directory.
- **Reference outputs.** Commands, from the repo root:

  ```bash
  git switch remove-pdal-gdal   # pre-rewrite code
  cargo build --release --locked
  cp -r out-poussebout out-reference-poussebout   # out-reference-poussebout must not exist yet
  cp target/release/cassini out-reference-poussebout/cassini-reference
  ./out-reference-poussebout/cassini-reference batch -o out-reference-poussebout --skip-lidar --ocad in-poussebout
  git switch -                  # back to the step branch
  ```

  To check a new binary: copy `out-poussebout` to the scratchpad, run the same batch on the copy with `./target/release/cassini`, then `compare_images out-reference-poussebout <copy>` (about 20 s per batch). Reference intermediates: `dem_with_buffer.tif`, `dem_2m_with_buffer.tif` and `slopes.tif` are Float32 (2800², 700², 2800²), uncompressed, nodata -9999; `*_vegetation_with_buffer.tif` are Byte 1400², nodata 255.

### Step 2 (branch `rust-pipeline/step-2-raster-ops`, cut from `remove-pdal-gdal` after the step 1 merge)

- **Parity: bit-identical.** On all four poussebout tiles, `compare_rasters` exits 0 for `dem_with_buffer.tif` (after `gdal_fillnodata`), the three `*_vegetation_with_buffer.tif`, `dem_2m_with_buffer.tif` and `slopes.tif`. `contours-raw.shp` and all 17 final PNGs are byte-identical too. `--skip-lidar --ocad` batch: 12.3 s vs 15.2 s for `cassini-reference`.
- **Buffer** ([buffer.rs](../../src/buffer.rs)): `create_tif_with_buffer::<T>(…, resolution)` replaces the `-ot` string with the output type (`f32` for the DEM, `u8` for vegetation). Sources are read as `T`, pasted in order (tile, then neighbours) with nearest neighbour at output cell centres; nodata is transparent and a later valid cell wins, like the VRT. The output nodata is the tile raster's (an error if it has none); uncovered cells are nodata. Output size is `round(extent / resolution)`, uncompressed like `gdal_translate`. The DEM extent bug (2 km overhang of nodata) is harmless here: the overhang is transparent. A missing neighbour raster is skipped with a warning, as `gdalbuildvrt` skips files it cannot open; the tile's own raster is required. The function returns the grid it writes, so vegetation no longer re-reads the three `*_with_buffer.tif` (the files are still written, for the parity checks and step 3). Errors name the output file and the tile.
- **2 m DEM** (`downsample_average` in [dem.rs](../../src/dem.rs)): `f64` sum of the valid cells of each 4×4 block, divided by their count, cast to `f32`. Matches GDAL 3.12 exactly. Tile extents come from rounded LAS header bounds, so the size need not be a multiple of 4: like GDAL, the output size is `round(size / 4)`, a remainder under half a block is dropped and a partial last block repeats the last row or column (GDAL's edge clamping, measured with impulse rasters; tested bit-exact on 5×5, 9×6 and 10×7). The factor is a constant with a compile-time check that it is an integer. `DEM_BLOCK_SIZE` is now `f64`, so it is the exact grid resolution for any value; `cliffs.rs` narrows it to `f32` for its pixel maths (no change at 0.5).
- **Slope** (`slopes_in_degrees`): Horn's window sums in `f32` (GDAL reads a Float32 DEM as `float`), then `f64` for `* (1/ewres)`, `sqrt`, `/ 8`, `atan`, `to_degrees`. Edges and cells with a nodata neighbour are -9999. Exact against GDAL 3.12 on Linux (both use glibc `atan`; other platforms may differ by an ulp).
- **DEM step order** is unchanged: buffer → `gdal_fillnodata` in place → read the filled DEM once → 2 m DEM → `gdal_contour` → slope. Step 3 can keep the filled `Grid<f32>` in memory instead of re-reading it.
- **Projection** ([projection.rs](../../src/projection.rs), in `lib.rs` next to `grid`): direct LCC formulas (Snyder), no new dependency, constants computed once in a `LazyLock`. Only `lambert93_to_wgs84` is used so far; `wgs84_to_lambert93` is for step 5 (OSM import). Within 1e-8° of `cs2cs` (PROJ 9.7) for 7 points across France, forward within 1e-5 m. The Overpass bbox is formatted with `{:.8}` to keep the query as `cs2cs -f %.8f` produced it.
- **Removed:** `gdalbuildvrt`, both `gdal_translate`, `gdaldem` and `cs2cs` calls, and the now-unused `Stage::Buffer` and `Stage::Download`. `checked_output_with_input` stays in `process.rs`: `lidar.rs` pipes the PDAL pipeline through it.

### Step 3 (branch `rust-pipeline/step-3-fill-contours`, cut from `remove-pdal-gdal` after the step 2 merge)

- **Parity: bit-identical.** On all four poussebout tiles, `compare_rasters` exits 0 for `dem_with_buffer.tif`, `dem_2m_with_buffer.tif` and `slopes.tif`; `contours-raw.shp`, `.shx` and `.prj` are byte-identical (same features, vertices and order), and so are all 17 final PNGs. The `.dbf` differs only in header byte 29, the language driver ID (GDAL 0x57, `dbase` crate 0xF0); field definitions and records are identical. `--skip-lidar --ocad` batch: 11.8 s (19.2 s for `cassini-reference`, which also predates step 2).
- **Fill** ([fill_nodata.rs](../../src/fill_nodata.rs)): a port of `GDALFillNodata` (GDAL 3.12, attribution kept), defaults of `gdal_fillnodata` (100 cells, inverse distance, no smoothing). Each nodata cell takes the inverse-distance mean of the nearest data cell in each of 4 quadrants, but within a column only the vertically nearest data cell (from a top-down and a bottom-up pass) is a candidate, and the column scan stops every 4 steps at the farthest quadrant distance. Filled cells never feed other cells. Arithmetic is reproduced exactly (`f64` sums, `f32` result). One exactness-preserving shortcut: cells with no candidate column within 100 cells are skipped (GDAL scans them for nothing). Speed on tile 907000_6454000 (2800², unfilled DEM rebuilt with the old `gdalbuildvrt` + `gdal_translate`): **0.40 s vs 1.9 s** for `gdal_fillnodata`. The DEM stays in memory from buffer to slope; `dem_with_buffer.tif` is written once, filled.
- **Contours** ([marching_squares.rs](../../src/marching_squares.rs)): **not the `contour` crate**, as the plan suggested. It has no nodata support, closes rings along the grid border, and its `smooth` flag is what enables linear interpolation (off means cell midpoints). GDAL extends lines half a cell into nodata (squares with nodata corners are split into sub-squares) and to the grid edge, and Pullautin's smoothing depends on vertex order and line length (lines under 15 vertices are dropped, over 101 thinned), so a port of GDAL's marching squares (`square.h`, `level_generator.h`, `segment_merger.h`, `contour.cpp`, MIT) was the way to parity. Only the line mode is ported (no polygons, no border segments). The ported details that matter for exactness: `fudge` (values within 1e-6 of a level moved up), whole squares interpolate from the side midpoint, the same saddle choice, levels from the band's valid min/max (a minimum on a level excludes it), and the segment merger's row-by-row join and emission order. `ID` from 0, `elev` Numeric(24,15), `ID` Numeric(8,0), the ESRI WKT `.prj` OGR writes. Readers don't depend on line direction (depressions are detected from the DEM), but it matches GDAL anyway. Downsample + contours + both writes take 0.09–0.19 s per tile; `gdal_contour` alone took 0.25 s.
- **Removed:** the `gdal_fillnodata` and `gdal_contour` calls, and the now-unused `Stage::Dem` and `Stage::Contours`. Remaining external calls: `pdal` (step 4) and `ogr2ogr` (step 5).
