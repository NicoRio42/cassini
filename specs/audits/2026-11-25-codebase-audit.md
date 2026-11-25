# Cassini codebase audit

Date: 2026-11-25 · Commit: `9b76840` (`develop`) · Version: 0.19.0 (unreleased: `upgrade` command)

## Executive summary

Cassini has changed a lot since [the 2026-08-25 architecture report](../research/architecture-and-maintainability-report.md). PDAL, GDAL and PROJ are gone. Errors go through `Result<_, CassiniError>`. The configuration is validated, and the test count went from 5 to 192. Most of the critical findings of that report are resolved.

What remains falls into three groups:

1. **Correctness bugs in pixel and georeferencing math.** Every renderer converts coordinates to pixels in its own way, and several of them get it wrong:
   - the contour renderer still hard-codes 600 DPI;
   - vector features are snapped to whole metres;
   - the merged map has 1-pixel seams;
   - world files are off by half a pixel and lose precision in `f32`.

   All of them come from one missing abstraction, a shared render transform.
2. **One data-loss hazard.** `process` and `lidar` empty their output directory before writing to it, whatever that directory contains.
3. **No quality gate.** No workflow runs the tests, `rustfmt` or Clippy. Formatting and Clippy both fail today, and the documentation has drifted from the code again.

The highest-value work, in order:

| # | Action | Effort | Fixes |
|---|---|---|---|
| 1 | Stop wiping arbitrary output directories | S | B2 |
| 2 | Use `config.dpi_resolution` in the contour renderer | S | B1 |
| 3 | Add a CI workflow (fmt, Clippy, tests, UI tests); fix the current fmt/Clippy failures | S | M1, M2 |
| 4 | Introduce a `RenderTransform` and use it in every renderer, the merge and the world files | M | B3, B4, B5, B9, A3 |
| 5 | Move every module into the library crate, group the modules into themed folders with a dependency direction, type the tile artifacts | M | A1, A10, A2 |
| 6 | Restructure `MapRenderer` around a symbol style table and fewer full-size layers | L | A4 |

## Scope and method

- Read the Rust crate (`src/`, about 26 k lines including tests), the web UI assets, the examples, the workflows, the Dockerfile and the reference docs.
- Ran `cargo fmt --all -- --check`, `cargo clippy --all-targets --locked`, `cargo test --all-targets --locked` and `node --test tests/ui.test.cjs`.
- Checked the suspected bugs against the code. The merged-map seam was reproduced numerically with `f32` arithmetic. None of the fixes below were applied.

### Tooling snapshot

| Check | Result |
|---|---|
| `cargo test --all-targets --locked` | Pass: 157 bin + 35 lib tests, 2 ignored (benchmarks) |
| `node --test tests/ui.test.cjs` | Pass: 6 tests |
| `cargo fmt --all -- --check` | **Fail**: 10 hunks in `grid.rs`, `vegetation.rs`, `examples/compare_images.rs`, `examples/compare_rasters.rs` |
| `cargo clippy --all-targets` | **Fail**: 1 deny-level error (`src/ui/job.rs:706`, test code) + 89 warnings, 69 auto-fixable |
| CI running any of the above | **None** |

### Progress since the 2026-08-25 report

| Finding (old report) | Status |
|---|---|
| 1. External failures do not propagate | **Resolved.** `CassiniError` + `ResultContext`, `main` prints the cause chain and exits non-zero. External tools removed. Only ~35 `unwrap`/`expect` remain outside tests, mostly mutex locks. |
| 2. Hard-coded 600 DPI in contours | **Still open**, see B1 |
| 3. Batch edge cases (0 threads, empty input, worker panics) | **Resolved.** `NonZeroUsize`, empty-input error, panics reported per tile |
| 4. Implicit artifact protocol / stale layers | **Partly resolved.** Stale optional layers are removed, `--skip-lidar` checks `vegetation_layers.txt` and the DEM grid. File names are still string literals spread over the modules (A2). |
| 5. Configuration global and unvalidated | **Resolved** except for `process`, which has no `--config` (B7) |
| 6. Rendering modules mix concerns | **Partly.** `MapFeature` classification now exists, but `MapRenderer` and the Pullautin port are unchanged (A4, A9) |
| 7. Memory and concurrency | **Open** (A4, A5) |
| 8. Networking boundaries | **Resolved** for Overpass (timeouts, transient-only retries, capped `Retry-After`, `.part` files). HTTP code is now duplicated four times (A6). |
| 9. Library API | **Open**, and the lib/bin split is now inside out (A1) |
| 10. Test and CI coverage | **Tests much better, CI still missing** (M1, M6) |
| 11. Documentation drift | **Still open**, see B8 and M5 |
| 12. Build reproducibility | **Resolved.** `--locked` check in dist, slim pinned Ubuntu image, no runtime GDAL |

## Bugs

Severity reflects user impact: **High** means wrong output or data loss in a supported configuration.

### B1. The contour layer ignores `dpi_resolution` (High)

[`pullautin_contours_render.rs:191`](../../src/pullautin_contours_render.rs) converts contour vertices to pixels with the literal `600.0 / 254.0`. So do lines 220–222 and 352–353. On line 41 of the same function, `buffer_in_pixels` uses `config.dpi_resolution`. The pixel-unit constants `curvew`, `dashlength`, `gaplength` and `minimumgap` are also tuned for 600 DPI.

`Config::validate` accepts any DPI from 0 to 2400. At any value other than 600, `contours.png` is drawn at the wrong scale and offset, and is misaligned with the vegetation, cliffs and vectors in `full-map.png`. No test renders at another DPI.

**Fix:** derive the scale from `config.dpi_resolution / INCH`, and scale the pixel constants by `dpi / 600`. Then add a smoke test at 300 DPI (see A3).

### B2. `process` and `lidar` delete everything in their output directory (High)

[`lidar.rs:261`](../../src/lidar.rs) calls `remove_dir_content_except(output_dir_path, &[osm_file])`. This empties the output directory except for the OSM file, and no check limits which directory it can be. The cleanup was added for issue #7.

As a result, `cassini process data/tile.laz -o data` or `cassini lidar tile.laz -o .` deletes every file and subdirectory there. That includes `config.json`, other LiDAR files and the input LAZ file itself. On Unix the open file handle keeps the read working, so the input is deleted silently.

`batch` is not affected, because it only cleans its own `out/<extent>` subdirectories.

**Fix:** remove only the files Cassini writes. A `LIDAR_STEP_FILES` list mirrors `RENDER_STEP_FILES` and should come from the same artifact type (A2). Alternatively, refuse to clean a non-empty directory that lacks a Cassini marker file such as `vegetation_layers.txt` or `extent.txt`.

### B3. The merged map has 1-pixel seams every few tiles (Medium)

The tiles and the merge round pixel positions differently:

- [`render.rs:68`](../../src/render.rs) truncates each tile to `floor(1000 m × 600 / 254) = 2362` px.
- [`merge.rs:109`](../../src/merge.rs) places each tile at `floor(offset × 600 / 254)`.

The exact width is 2362.205 px, so the truncation error builds up. Simulated in `f32`:

| Tile index | Placed at | Previous tile ends at | |
|---|---|---|---|
| 4 | 9448 | 9448 | |
| 5 | 11811 | 11810 | **1 px gap** |
| 10 | 23622 | 23621 | **1 px gap** |

At 600 DPI, a transparent column or row appears every 5 tiles (5 km) in `merged-map*.png`. Other DPIs give other periods.

**Fix:** compute each tile's pixel rectangle on the batch's global pixel grid as `[round(x0·s), round(x1·s))`, and render the tile at exactly that size. The `RenderTransform` (A3) is the right place for this. Add a unit test checking that adjacent tile rectangles touch.

### B4. Vector features are snapped to whole metres (Medium)

[`map_renderer.rs:456`](../../src/map_renderer.rs) computes `(point.x as i64 - self.min_x) as f32 * self.scale_factor`. The `as i64` truncates every OSM or shapefile vertex to the metre below it before scaling. At 600 DPI that is an error of up to 2.4 px, which shows on building outlines, narrow paths and short segments, and it grows with DPI. The OCAD export uses the unrounded `f64` coordinates, so the PNG and the OCD file disagree.

**Fix:** `((point.x - self.min_x as f64) * scale) as f32`, and the same for y.

### B5. World files lose precision and are off by half a pixel (Medium)

There are three separate problems:

- **Wrong origin:** [`world_file.rs`](../../src/world_file.rs) writes the tile's top-left corner as the reference point. The world-file convention (lines 5–6) is the centre of the top-left pixel, so `full-map.pgw` and `merged-map*.pgw` are shifted by half a pixel: 0.21 m at 600 DPI, more at lower DPI. Check this against how QGIS and OCAD georeference the PNG before changing it.
- **`f32` precision:** `create_world_file` takes `f32`, and [`merge.rs:82`](../../src/merge.rs) computes the chunk bounds in `f32`. At Lambert-93 northings (6–7 × 10⁶ m) the `f32` resolution is 0.5 m, so the world file of every chunk below the first row can be off by up to 0.25 m.
- **Pixel size mismatch:** the chunk origin is `min + px / total_width × geo_width`. `total_width` is a `ceil`, so the ground size of a pixel implied by the origins differs from the `resolution` written on line 1. Use `min + px × resolution`.

**Fix:** use `f64` throughout, take origins from the transform (A3), and add a world-file unit test.

### B6. Batch input discovery is inconsistent and fragile (Medium)

[`batch.rs:383`](../../src/batch.rs) accepts only the exact extension `laz` and does not skip hidden files. `geotiff_paths` does both, a few lines below. As a result:

- **macOS AppleDouble files** (`._LHD_….laz`, written on exFAT and network drives) pass the filter. `read_tile_id` then fails on them and aborts the whole batch. The DEM discovery already skips them.
- **`.LAZ` files** are silently ignored by `batch`. The web UI's [`laz_files_in`](../../src/ui/mod.rs) matches case-insensitively, so it lists them as present.

**Fix:** one `is_input_file(path, extensions)` helper, used by both discoveries and the UI.

### B7. `process` cannot take a configuration file (Low)

`process` always reads `./config.json` ([`main.rs:335`](../../src/main.rs)), while `lidar`, `render` and `batch` take `-C/--config`. The docs only promise the option for those three, but there is no reason for `process` to differ.

`lidar --config` loads and validates the file, then discards it, because the LiDAR stage uses no configuration. Either drop the option or say so in its help.

### B8. Documentation contradicts the code (Low)

- [Configuration reference](../../docs/src/content/docs/reference/configuration-reference.mdx): documents green thresholds 1/2/3 as defaulting to `1.0`/`2.0`/`3.0`. [`config.rs:11-13`](../../src/config.rs) uses 2/4/6.
- [CLI reference, line 454](../../docs/src/content/docs/reference/cli-reference.mdx): shows `cassini 0.4.0` for `--version`.

The `// Update the docs when modifying` comments have not prevented this. See M5.

### B9. Raster cells are drawn at their corner instead of their centre (Low, verify)

- **Cliffs:** [`cliffs.rs:39`](../../src/cliffs.rs) centres each cliff dot on the top-left corner of its 0.5 m slope cell, which shifts the cliffs 0.25 m south-east.
- **High vegetation:** [`vegetation.rs`](../../src/vegetation.rs) centres the white ellipse of each 1 m cell on its corner as well.

Both are sub-metre, but they stack up with B4. The OCAD export uses cell centres (`Window::center`), so the PNG and the OCD file disagree slightly. Check against a reference render before changing it, since the vegetation look was tuned visually.

### B10. The Overpass bounding box does not follow the Lambert-93 grid (Low)

[`download.rs:56`](../../src/download.rs) transforms only the south-west and north-east corners to WGS 84. The Lambert-93 grid is rotated against meridians, by up to about 3° at the edges of France. The transformed rectangle therefore misses part of the other two corners: about 1.4 km × tan(3°) ≈ 75 m. The 200 m buffer absorbs it, so the tile itself is covered, but the effective buffer is smaller at two corners.

**Fix:** transform all four corners and take the min/max.

### B11. Batch output order is not deterministic (Low)

`get_tiles_with_neighbors` builds its tile list from a `HashMap`, so the tile order changes between runs. That order decides:

- the order of the objects in `merged-map.ocd`;
- the log order;
- the tie-breaking in `ocad_merge`'s candidate sort, which sorts by distance only.

Two identical runs can produce byte-different OCD files, which makes regression comparison harder. Sort the tiles by `(min_x, min_y)`.

### B12. Coastlines are processed in `f32` world coordinates (Low)

[`vectors.rs:267`](../../src/vectors.rs) and line 430, [`coastlines.rs`](../../src/coastlines.rs) and [`helpers.rs`](../../src/helpers.rs) store Lambert-93 coordinates in `f32`, which has a 0.5 m step at those northings. The impact is limited to coastlines and the tile-intersection tests, but the fix is mechanical: use `f64`, or tile-relative coordinates.

### B13. A NaN can panic the area vectorization (Low)

[`vectorization.rs:317`](../../src/vectorization.rs) sorts with `partial_cmp(..).unwrap()`. Use `total_cmp`.

## Architecture

### A1. The library/binary split is inside out

[`lib.rs`](../../src/lib.rs) exposes 5 modules (`error`, `grid`, `osm_import`, `projection`, `shapefile_output`) for the examples. [`main.rs`](../../src/main.rs) declares the other 37 as private binary modules. It then re-imports the library modules (`use cassini::{error, grid, projection}`) so that `crate::grid` resolves inside them.

This has several consequences:

- Modules mix `crate::` paths with `cassini::` paths (`vectors.rs`, `contour_shapefile.rs`).
- The UI depends on the binary root: [`ui/job.rs:35`](../../src/ui/job.rs) imports `crate::DEFAULT_OCAD_SIMPLIFY_TOLERANCE` from `main.rs`.
- Nothing in `tests/` can exercise the pipeline. Every test has to live inside the module it tests.
- `main.rs` is 900 lines, combining CLI definitions, the command dispatch and the step functions.

**Recommendation:**

- Move every module into the library, as `pub(crate)` by default, with a small public facade for what the examples and integration tests need.
- Split `main.rs` into a `cli.rs` (clap types) and a `commands.rs` (dispatch).
- Keep `main.rs` to logger setup, `run()` and `report_error`.

### A2. Stage outputs are still an implicit file protocol

File names are string literals scattered across modules: `"slopes.tif"`, `"contours.png"`, `"cliffs.png"`, `"full-map.png"`, `"undergrowth.png"`, `"extent.txt"`, `"dem.tif"`, `"<name>_with_buffer.tif"`. The OSM file name format `{:0>7}_{:0>7}.osm` alone is repeated in 5 places. `RENDER_STEP_FILES` in [`render.rs`](../../src/render.rs) is a hand-maintained list of 17 entries that must match what the other modules write. B2 exists because no equivalent list exists for the LiDAR stage.

**Recommendation:** add a `TileArtifacts` type (`tile.rs` is the natural home) with one method per artifact (`dem()`, `slopes()`, `osm()`, `full_map_png()`, …) and the per-stage lists derived from it. Pass it instead of `Tile` plus string joins. A versioned manifest, as the old report proposed, can come later. The `vegetation_layers.txt` marker shows the need is real.

The renderers also communicate through PNG files: vegetation, contours, cliffs and undergrowth are drawn with `image`/`imageproc`, encoded to PNG, then decoded into Skia by `MapRenderer`. That is two raster libraries and a full encode/decode of up to four full-size images per tile. The [performance report](../research/render-pipeline-performance-report.md), section 6, already quantifies this. Architecturally, the renderers should return in-memory layers and let only `full-map.png` reach disk, keeping the intermediate PNGs behind a debug flag.

### A3. There is no shared render transform

Pixel math is copied everywhere:

- 40 occurrences of `x * dpi_resolution * 10.0 / INCH` or `/ INCH`;
- separate `as i32`/`floor`/`ceil` choices in each renderer;
- a different handling of the 200 m buffer in each module.

`INCH = 254.0` actually encodes both the inch and the 1:10 000 map scale, which the name hides. B1, B3, B4, B5 and B9 all come from this.

**Recommendation:** one `RenderTransform`, built once per tile from the tile extent, the DPI and the batch's global pixel grid. It would provide:

- `world_to_px(x, y) -> (f32, f32)`, in `f64` internally;
- `mm_to_px(mm)` for symbol widths, replacing the `* 10.0 / INCH` copies;
- `cell_center_px(col, row, resolution, origin)` for raster layers;
- `pixel_rect()` and `world_file()`, so the tile size, the merge offsets and the georeferencing share one definition.

Replace the `"Tile min_x={} min_y={} max_x={} max_y={}. …"` log prefix (34 copies) with the existing `Display for TileId`, or with a logging span set once per tile.

### A4. `MapRenderer` holds 14 full-size layers and one method per symbol

[`map_renderer.rs`](../../src/map_renderer.rs) allocates or loads 14 RGBA canvases up front. At 600 DPI each one is about 22 MB, so about 310 MB per tile, and about 1 GB with the default 3 batch threads, before vegetation and DEM buffers. The API is by value (`fn symbol(mut self, …) -> MapRenderer`), with one method per ISOM symbol, plus a 60-line `match` in [`vectors.rs`](../../src/vectors.rs) mapping `MapFeature` to those methods. Adding a symbol means editing `constants.rs`, `map_renderer.rs`, `vectors.rs`, `map_features.rs` and `ocad_export.rs`.

**Recommendation:**

- Describe each symbol as data in a style table: target layer, colour, widths, dash pattern, fill or stroke. Make the renderer a loop over features grouped by layer, drawing one layer at a time and compositing it into the result immediately.
- Allocate a layer only when a feature needs it.
- Take `&mut self`.

This also gives the OCAD symbol mapping and the PNG styling a single list of symbols.

### A5. Batch scheduling is static and memory-unaware

`chunk_size = tiles.len().div_ceil(threads)` ([`batch.rs:74`](../../src/batch.rs)) assigns fixed chunks up front. With 10 tiles and 3 threads that is 4/4/2, and a slow tile stalls its whole chunk. LAZ decompression already uses every core, as the `--threads` help says, so the effective parallelism is unclear.

The neighbour lookup is 8 hand-written calls with exact `(min_x, min_y, max_x, max_y)` keys, which only works for equally sized, aligned tiles. That is fine for IGN, but nothing checks the assumption.

**Recommendation:**

- Replace the chunks with a shared work queue, an `Arc<Mutex<IntoIter>>` drained by N workers, or a `rayon` pool of size N.
- Compute the neighbours with a loop over the 8 offsets.
- Warn when the tiles are not all the same size.

### A6. HTTP code is duplicated four times

[`download.rs`](../../src/download.rs) (Overpass), [`ui/job.rs`](../../src/ui/job.rs) (LAZ and DEM downloads), [`ui/ign.rs`](../../src/ui/ign.rs) (IGN index) and [`update.rs`](../../src/update.rs) (GitHub) each build their own `reqwest` client with their own timeouts. Two of them contain their own retry loop and `.part` file handling. `ui/job.rs` already borrows `is_transient_status` and `retry_delay_from_response` from `download.rs`, which shows the shared module that is missing.

**Recommendation:** an `http` module with:

- the client factory (user agent, timeouts);
- `retry(policy, attempt)`;
- `download_to(path, url, progress, cancel)`, with `.part` files, optional `Range` resume for the multi-hundred-MB LAZ files, and a `Content-Length` check.

### A7. The web UI duplicates the CLI options as strings

`BatchOptions` in [`ui/job.rs`](../../src/ui/job.rs) re-declares the `batch` options as strings and rebuilds the command-line arguments by hand. The defaults are duplicated:

- `DEFAULT_THREADS = 3` in `job.rs`, `default_value = "3"` and `.unwrap_or(3)` in `main.rs`;
- `DEFAULT_OCAD_SIMPLIFY_TOLERANCE` is imported from the binary root (A1).

Adding a `batch` option means editing the clap definition, `BatchOptions`, `BatchOptions::args`, `app.js` and the UI tests.

**Recommendation:** have the clap `Args` structs for `batch` derive `Serialize`/`Deserialize`, or generate `BatchOptions` and `choices()` from `Commands::Batch` via `clap::CommandFactory`. Keep each default in one constant.

### A8. The CLI and the internal APIs are hard to read

- `Option<String>` arguments with `default_value` are always `Some`. Each one is then unwrapped with the same default repeated (`maybe_output_dir.unwrap_or("tile".to_owned())`, `unwrap_or(3)`). Use `PathBuf` and `NonZeroUsize` fields directly.
- Paths are passed as `&str`/`String` in `batch` and `merge`, and as `PathBuf` elsewhere. Use `&Path` everywhere.
- Long positional parameter lists, and a bare `bool` (`generate_dem_and_vegetation_density_tiff_images_from_laz_file(…, true)`):

  | Function | Parameters |
  |---|---|
  | `MapRenderer::new` | 10 |
  | `render_map_with_osm_vector_shapes` | 9 |
  | `batch` | 7 |

  Introduce request structs such as `BatchRequest` and `RenderRequest`, and an enum instead of the bool.
- `std::env::args().len() == 1` after `Args::parse()` reimplements clap's `arg_required_else_help`.

### A9. The Pullautin port is one 470-line function

[`pullautin_contours_render.rs`](../../src/pullautin_contours_render.rs) takes `&Vec<Vec<f64>>`, keeps Karttapullautin's names (`help2`, `curvew`, `sxmax`, `formlineaddition`) and inline constants, and inside the same loop:

- computes the steepness;
- culls form lines;
- writes a shapefile;
- rasterizes the contours, each stroke as an n×m grid of 1-px segments (performance report, section 8);
- saves a PNG.

**Recommendation:** keep the numerical algorithm intact, but split it into:

- `steepness(avg_alt) -> Grid<f64>`;
- `cull_formlines(contours, steepness) -> Vec<ContourPiece>`, a pure function that can be tested against the current output;
- rendering through the transform (A3);
- the shapefile writing, left to the caller.

Name the constants and document their units, which are pixels at 600 DPI today (B1).

### A10. 42 flat modules: group them by theme, with dependency rules

Apart from `osm_import/` and `ui/`, `src/` has 40 modules in one directory. The names already form families that the layout doesn't show:

- `vegetation` / `vegetation_areas`;
- `cliffs` / `cliff_features`;
- `contours` / `contour_shapefile` / `pullautin_smooth_contours` / `pullautin_contours_render` / `marching_squares`;
- `ocd` / `ocad_export` / `ocad_merge`;
- `vectors` / `vectorization`, which are unrelated despite their names.

Several modules mix themes:

- [`dem.rs`](../../src/dem.rs) holds the LiDAR-stage `dem_with_buffer` functions and the render-stage contour and slope extraction.
- [`helpers.rs`](../../src/helpers.rs) mixes filesystem cleanup with segment intersection.
- [`constants.rs`](../../src/constants.rs) mixes grid sizes with colours and ISOM symbol widths.
- [`vectors.rs`](../../src/vectors.rs) both classifies OSM features and drives `MapRenderer` (`render_map_with_osm_vector_shapes`).

Grouping these into folders is worth doing. Folders alone are cosmetic, though: the value comes from a **dependency direction** that the folders make visible and reviewable. Today the dependencies run in every direction:

| Edge | Problem |
|---|---|
| `lidar` → `dem` | The LiDAR stage depends on a render-stage module for `fill_dem_with_buffer` and `write_dem_with_buffer`. |
| `vectors` → `map_renderer` | A feature source depends on the PNG output. |
| `update` → `download` | The upgrade command borrows the Overpass module's user agent. |
| `ui::job` → `main.rs` | It needs `DEFAULT_OCAD_SIMPLIFY_TOLERANCE`, which lives in the binary root (A1, A7). |
| `render` → 15 modules | Expected for an orchestrator, but it lives next to the modules it orchestrates. |

#### Proposed layout

Each map layer (contours, cliffs, vegetation, vectors) becomes one folder. Each folder computes its layer from the rasters, draws it for the PNG, and vectorizes it for OCAD. Those three are what changes together when a layer is tuned.

```text
src/
├── main.rs                   logger, run(), report_error
├── lib.rs
├── error.rs  config.rs
├── tile.rs                   Tile, TileId, TileArtifacts (A2)
├── constants.rs              grid sizes only: DEM_BLOCK_SIZE, VEGETATION_BLOCK_SIZE, BUFFER
├── cli/
│   ├── mod.rs                ← main.rs: Args, Commands, VegetationArgs, OcadArgs
│   └── commands.rs           ← main.rs: process, lidar, render, batch steps
├── geo/
│   ├── projection.rs
│   ├── world_file.rs
│   ├── transform.rs          new: RenderTransform (A3)
│   └── geometry.rs           ← helpers.rs: segment and polyline intersection
├── raster/
│   ├── grid.rs               Grid and GeoTIFF I/O
│   ├── mosaic.rs             ← buffer.rs
│   ├── fill_nodata.rs
│   ├── marching_squares.rs
│   └── trace.rs              ← vectorization.rs
├── formats/
│   ├── las.rs                ← point_reader.rs
│   ├── shapefile.rs          ← shapefile_output.rs
│   └── osm/                  ← osm_import/
├── net/
│   ├── http.rs               ← download.rs: client, retries, .part downloads (A6)
│   └── overpass.rs           ← download.rs
├── lidar/
│   ├── mod.rs                ← lidar.rs: run_dem_pass, VegetationByHeightPass
│   ├── ground_dem.rs         ← lidar.rs: GroundDem, TileCells
│   ├── densities.rs          ← lidar.rs: VegetationDensities, VoxelSet
│   ├── external_dem.rs
│   └── dem_with_buffer.rs    ← dem.rs: create, fill, read, check
├── contours/
│   ├── mod.rs                ← contours.rs
│   ├── extract.rs            ← dem.rs: 2 m DEM and raw contours
│   ├── shapefile.rs          ← contour_shapefile.rs
│   ├── smooth.rs             ← pullautin_smooth_contours.rs
│   └── formlines.rs          ← pullautin_contours_render.rs, split per A9
├── cliffs/
│   ├── slopes.rs             ← dem.rs: slopes
│   ├── raster.rs             ← cliffs.rs
│   └── features.rs           ← cliff_features.rs
├── vegetation/
│   ├── mod.rs                ← vegetation.rs: VegetationRasters, classification
│   ├── raster.rs             ← vegetation.rs: render_vegetation
│   └── areas.rs              ← vegetation_areas.rs
├── vectors/
│   ├── mod.rs                ← vectors.rs: OSM and shapefile features → MapFeature
│   └── coastlines.rs
├── map/
│   ├── features.rs           ← map_features.rs: ISOM symbols
│   ├── style.rs              ← constants.rs: colours, symbol widths (A4 style table)
│   ├── canvas.rs
│   └── renderer.rs           ← map_renderer.rs + render_map_with_osm_vector_shapes
├── ocad/
│   ├── ocd.rs                OCD 12 reader and writer
│   ├── export.rs             ← ocad_export.rs
│   └── merge.rs              ← ocad_merge.rs
├── pipeline/
│   ├── render.rs             ← render.rs: one tile
│   ├── batch.rs
│   ├── merge.rs              ← merge.rs: merged PNG
│   └── options.rs            RenderOptions, BatchOptions shared by the CLI and the UI (A7)
├── ui/                       unchanged
└── update.rs
```

The remaining fs helpers in `helpers.rs` (`remove_if_exists`, `remove_dir_content_except`) belong with `TileArtifacts` in `tile.rs`, or in a small `fs.rs`.

#### Dependency rules

Each level may depend only on the levels above it:

1. **Foundation:** `error`, `config`, `tile`, `constants`, `geo`, `raster`, `formats`, `net`. These know nothing about maps.
2. **`lidar`:** foundation only. This means `dem_with_buffer` moves into it.
3. **`map/features`:** the ISOM symbol vocabulary shared by the layers and the outputs.
4. **Layers** (`contours`, `cliffs`, `vegetation`, `vectors`): foundation and `map/features`. They never depend on each other or on an output.
5. **Outputs** (`map/renderer`, `ocad`): foundation and `map/features`. They take features and layers as input and don't know where they come from.
6. **`pipeline`:** everything above.
7. **`cli`, `ui`, `update`:** `pipeline` and the foundation. `ui` never imports from `cli` or `main`.

With the folders in place, a rule violation shows up as a `use crate::pipeline::…` inside `raster/`, which a reviewer can see. If that is not enough, `cargo-modules` or a short test that scans the `use` lines can enforce the rules.

#### Migration

- **Do A1 first.** Once every module is in the library, the moves only change paths inside one crate.
- **Move one folder per commit, with `git mv`.** Make each commit path changes only, with no logic changes and a passing test suite, so that `git log --follow` and `git blame` keep working. Start with the foundation folders, then `lidar`, the layers, the outputs and `pipeline`.
- **Split the mixed modules in separate commits from the moves:** `dem.rs`, `helpers.rs`, `constants.rs`, `vectors.rs`, `main.rs`.
- **Schedule it between plan steps.** The plans in `specs/plans/` link to `src/*.rs` paths, and any open branch will conflict. Update the links in the same change.

## Maintainability and tech debt

### M1. No CI quality gate

`release.yml` runs on pull requests, but only plans and builds with dist. No workflow runs `cargo test`, `rustfmt`, Clippy or the Node UI tests, so the 192 tests protect only the machine they run on.

**Recommendation:** a `ci.yml` on push and pull request, on `ubuntu-22.04`, with `libfontconfig1-dev libfreetype6-dev`:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
node --test tests/ui.test.cjs
```

Add a build-only job on macOS and Windows if CI minutes allow, since Skia linking is the usual breaker there.

### M2. Formatting and Clippy currently fail

- **Formatting:** `cargo fmt --all` fixes it in one commit.
- **Clippy, the deny-level error:** `unused_io_amount` at `src/ui/job.rs:706`. This test helper reads an HTTP request with one `read()` call and can see a partial request, a real source of flaky tests. Use a `BufReader` and read up to the blank line.
- **Clippy, the 89 warnings:**

  | Warning | Count |
  |---|---|
  | Needless `return` | 34 |
  | Same-type casts (`f32 as f32`, `f64 as f64`) | 15 |
  | `&Vec<T>` parameters | 5 |
  | Very complex types | 4 |
  | Too many arguments | 2 |
  | Index loops | 8 |

  69 of them are auto-fixable with `cargo clippy --fix`. Fix them once, then enforce `-D warnings` in CI.

### M3. Dead code, naming and leftovers

- [`constants.rs`](../../src/constants.rs): underscore-prefixed unused constants (`_CONTOUR_THICKNESS_MILLIMETTER`, `_MAJOR_POWERLINE_*`, `_FORM_CONTOUR_*`, `_DOUBLE_TRACK_WIDE_ROAD_CENTRAL_WIDTH`).
- [`canvas.rs`](../../src/canvas.rs): unused methods `_set_stroke_cap_round` and `_unset_stroke_cap`.
- An empty `// TODO` at `map_renderer.rs:269`, and a stale TODO about `img.save` at `pullautin_contours_render.rs:486`.
- Two unrelated `CONTOUR_INTERVAL` constants: 2.5 m extraction in `dem.rs`, 5 m symbol interval in `pullautin_contours_render.rs`. Rename them (`CONTOUR_EXTRACTION_INTERVAL`, `MAP_CONTOUR_INTERVAL`).
- Misspellings that leak into identifiers: `algorithme`, `MILLIMETTER`, `INCROSSABLE`, `scalefactor`.
- `INCH` (see A3), and `COASTLINE_EDGE_BUFFER`'s comment, which states an invariant ("should be lower than osm download buffer") that a `const _: () = assert!(…)` could enforce, as `cliff_features.rs` already does for `MARGIN < BUFFER`.

### M4. Dependency hygiene

- `thiserror` 1.x (2.x is current).
- `skia-safe` 0.75 is several releases behind, and each bump is heavy because of prebuilt binaries. Plan it rather than drift.
- No `cargo-deny` or `cargo-audit`. Add `cargo-deny` (advisories, licenses, duplicate versions) to CI, since the release ships a statically linked binary.

### M5. Generated, not hand-copied, reference docs

The CLI reference and the configuration defaults are copied by hand, and they have drifted again (B8).

**Recommendation:**

- Generate the CLI usage blocks from clap (`clap-markdown`, or a small `examples/print_cli_reference.rs`).
- Generate the defaults table from `Config::default()`.
- Add a test that fails when the checked-in docs differ.

Also add a `CLAUDE.md` or `CONTRIBUTING.md` with the build prerequisites, the test commands (Rust and Node) and the end-to-end procedure: copy an `out-*` directory and run `batch --skip-lidar`. Today that procedure lives only in the plan notes.

### M6. Test gaps

Coverage is good in the algorithmic modules: `grid`, `lidar`, `fill_nodata`, `marching_squares`, `cliff_features`, `ocad_merge`, `osm_import`, `update`. It is absent where the bugs above are:

| Untested | Would have caught |
|---|---|
| `merge.rs` (tile placement, chunking, world files) | B3, B5 |
| `world_file.rs` | B5 |
| `map_renderer.rs` coordinate conversion | B4 |
| Any render at DPI ≠ 600 | B1 |
| `lidar`/`process` output directory handling | B2 |
| LAZ discovery with hidden and upper-case files | B6 |

The 10 m synthetic tiles in `lidar::test_tiles` are a ready fixture. A `render_tile_map` smoke test on two adjacent synthetic tiles with `skip_vector`, at 300 and 600 DPI, followed by `merge_maps`, would cover most of this in under a second. A golden-image comparison can reuse `examples/compare_images.rs`.

## Security notes (web UI and upgrade)

No significant issue was found.

- **What is right:** `cassini ui` binds to 127.0.0.1 and checks the `Host` header against DNS rebinding. It requires a random token on every API call, and it only serves `out/<int>_<int>_<int>_<int>/full-map.png`, which rules out path traversal.
- **The token:** it comes from two `RandomState` SipHash outputs. That is acceptable, since the keys come from the OS RNG, but `getrandom` would state the intent.
- **`config.json`:** a job overwrites the user's `config.json` without a backup.
- **Upgrade:** `cassini upgrade` runs the release's install script, fetched over HTTPS from GitHub, without checking a checksum or signature. That is the same trust as the documented `curl | sh` install. If dist publishes checksums for the installers, checking them would be cheap.

## Recommended sequence

### Phase 1: quick correctness and gating (about a day)

- B2: restrict the cleanup to known artifacts, and add a regression test.
- B1: use the DPI in the contour renderer, and add a 300 DPI smoke test.
- B4, B6, B8, B13: small, local fixes.
- M2: run `cargo fmt`, apply `clippy --fix`, fix the `job.rs` test helper.
- M1: add `ci.yml`.

### Phase 2: one render transform

- A3: `RenderTransform`, used by vegetation, cliffs, contours, `MapRenderer`, the merge and the world files.
- B3, B5, B9: fixed through it, with unit tests on tile rectangles and world files.
- B11: sort the batch tiles.

### Phase 3: structure

- A1: move all modules into the library and split `main.rs`.
- A10: group the modules into themed folders, one `git mv` commit per folder, then split `dem.rs`, `helpers.rs`, `constants.rs` and `vectors.rs`.
- A2: `TileArtifacts`, used to derive the cleanup lists.
- A6: `http` module.
- A7, A8: shared CLI and UI option definitions, request structs.
- M5: generated reference docs.

### Phase 4: renderer and scheduling

- A4: style table, layers allocated and composited one at a time, no intermediate PNGs (performance report, sections 4 and 6).
- A5: work-queue scheduler.
- A9: split the Pullautin port into pure steps.
- M4: dependency upgrades with `cargo-deny`.

Phases 1 and 2 change the rendered output only where they fix a bug, and phases 3 and 4 should not change it at all. Compare against the reference `out-*` renders with `examples/compare_images.rs` before merging, as the earlier plans did.
