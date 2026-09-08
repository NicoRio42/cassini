# Cassini architecture and maintainability report

Date: 2026-08-25

## Executive summary

Cassini has a clear domain pipeline and sensible file-level separation, but its orchestration is fragile. Most stages communicate through implicitly named files, return `()`, and use panics or logging for failures. This makes partial output, stale artifacts, and misleading downstream errors likely.

The highest-priority improvements are:

1. Introduce structured error propagation and stop immediately when external tools fail.
2. Fix configuration-dependent rendering correctness, especially hard-coded 600 DPI contour calculations.
3. Validate batch arguments, configuration, extents, and intermediate artifacts.
4. Make pipeline stages transactional and explicitly describe their inputs and outputs.
5. Add integration and golden-image tests plus CI checks before substantially restructuring algorithms.

## Scope and method

This report covers the Rust crate, its external PDAL/GDAL/PROJ/Overpass integrations, batch processing, rendering, tests, release automation, Docker packaging, and the separate Astro documentation site. The review was read-only apart from creating this report.

The repository was also checked with:

- `cargo test --all-targets`: passes; five tests ran.
- `cargo fmt --all -- --check`: fails because the current source is not fully formatted.
- `cargo clippy --all-targets --all-features -- -D warnings`: fails with 164 diagnostics, including 12 dead-code errors and numerous API/style warnings.

## Current architecture

The repository contains one Rust package with both library and binary targets, plus a separate Astro documentation site.

```text
CLI
 └─ library facade
     ├─ LiDAR preprocessing
     │   └─ PDAL → DEM and vegetation TIFFs + extent.txt
     ├─ vector acquisition
     │   └─ PROJ/cs2cs → Overpass → OSM → ogr2ogr shapefiles
     ├─ tile rendering
     │   ├─ GDAL buffer mosaics
     │   ├─ vegetation rasterization
     │   ├─ DEM, slope and contour generation
     │   ├─ cliffs and vector classification
     │   └─ Skia layer composition → PNG + world file
     └─ batch merge
         └─ geographically positioned tile PNGs → merged chunks
```

The major components are:

- [`src/main.rs`](../src/main.rs): CLI parsing and command dispatch.
- [`src/lib.rs`](../src/lib.rs): public facade for single-tile, staged, and batch operations.
- [`src/lidar.rs`](../src/lidar.rs): constructs and executes the PDAL pipeline.
- [`src/render.rs`](../src/render.rs): top-level render orchestrator.
- [`src/buffer.rs`](../src/buffer.rs) and [`src/dem.rs`](../src/dem.rs): GDAL-based raster preparation.
- [`src/vegetation.rs`](../src/vegetation.rs), [`src/cliffs.rs`](../src/cliffs.rs), and the Pullautin modules: domain algorithms.
- [`src/vectors.rs`](../src/vectors.rs) and [`src/map_renderer.rs`](../src/map_renderer.rs): feature classification and compositing.
- [`src/batch.rs`](../src/batch.rs): discovery, neighbor calculation, threading, and merging.
- [`src/tile.rs`](../src/tile.rs): the central but minimal data model.

### Architectural strengths

- Pipeline stages are recognizable and mostly separated by responsibility.
- Expensive LiDAR preprocessing can be reused independently of rendering.
- Neighbor buffers explicitly address tile-edge artifacts.
- External operations are logged with tile coordinates.
- Overpass retry handling includes `Retry-After` support.
- Merged images are split to bound individual Skia surface dimensions.
- Complex contour logic already has a small beginning of focused unit testing.
- `Cargo.lock` is committed, supporting repeatable Rust dependency resolution.

## Findings and recommendations

### 1. External failures do not propagate correctly

Priority: critical

The source contains approximately 70 `unwrap` calls, 52 `expect` calls, and 12 explicit panics. Public operations almost universally return `()`.

More importantly, subprocess failures are often logged and execution continues. PDAL failure is logged in [`src/lidar.rs`](../src/lidar.rs), after which the caller can proceed into rendering. GDAL commands behave similarly in [`src/buffer.rs`](../src/buffer.rs) and [`src/dem.rs`](../src/dem.rs). The eventual error will commonly be a missing-file or decoder panic rather than the original command failure.

Recommended changes:

- Define a `CassiniError` containing stage, tile, command, exit status, and stderr context.
- Make public and stage APIs return `Result<T, CassiniError>`.
- Centralize subprocess execution in a helper that checks the exit status and required outputs.
- Let `main` print one causal error chain and exit non-zero.
- In batch mode, collect per-tile failures and do not merge incomplete results.

### 2. Configurable DPI is not consistently honored

Priority: critical

Image dimensions and most symbols use `config.dpi_resolution`, but contour coordinates repeatedly use the literal `600.0` in [`src/pullautin_contours_render.rs`](../src/pullautin_contours_render.rs). At non-default DPI, contour positions, widths, gaps, buffer offsets, and the configured canvas can disagree.

Move coordinate conversion into a tested `RenderTransform` type containing extent, DPI, scale, buffer, and Y-axis orientation. All renderers should use that single transform.

### 3. Batch mode has unhandled edge cases

Priority: critical

The chunk calculation in [`src/batch.rs`](../src/batch.rs) fails when:

- `--threads 0` causes division by zero.
- No LAZ files are found, producing `chunks(0)`.
- A worker panics, where `join().unwrap()` loses useful tile and stage context.

The neighbor algorithm also relies on rounded, exactly matching, equally sized extents. Misaligned, overlapping, duplicated, or differently sized tiles can be silently excluded or overwritten in the `HashMap`.

Use `NonZeroUsize`, return an explicit empty-input error or successful no-op, validate extents, and model adjacency using spatial overlap or a documented tolerance rather than exact tuple arithmetic.

### 4. Intermediate artifacts are an implicit, unsafe protocol

Priority: critical

Stages exchange files such as `extent.txt`, `dem.tif`, `slopes.tif`, and `contours.png` by naming convention. There is no schema version, completeness marker, configuration hash, or input provenance.

Consequences include:

- `--skip-lidar` trusts whatever files happen to exist.
- Configuration or algorithm changes can reuse incompatible artifacts.
- The LiDAR stage clears the output directory before new processing succeeds.
- Failed stages can leave partial files that look reusable.
- Render-only operation does not consistently clear stale outputs.

There is a concrete stale-layer risk: only undergrowth mode 409 writes `undergrowth.png` in [`src/vegetation.rs`](../src/vegetation.rs), while the compositor enables undergrowth solely based on whether that file exists in [`src/map_renderer.rs`](../src/map_renderer.rs). Reusing an output directory with another mode can therefore overlay an old undergrowth layer.

Recommended changes:

- Introduce `LidarArtifacts`, `RenderArtifacts`, and a versioned JSON manifest.
- Include input identity, extent, CRS, Cassini version, relevant configuration hash, and completed stages.
- Write into a temporary per-run directory, validate outputs, then atomically rename it into place.
- Select optional layers from current run options, never from file existence alone.

### 5. Configuration is global and unvalidated

Priority: high

[`src/config.rs`](../src/config.rs) always reads `./config.json`. A missing file silently means defaults, malformed JSON panics, unknown keys are accepted, and values receive no semantic validation.

Validation should cover:

- Positive, bounded DPI.
- Ordered vegetation and cliff thresholds.
- Finite numeric values.
- Valid extents and raster dimensions.
- Resource limits derived from expected image size.

The equal default cliff thresholds are especially suspicious: because the large-cliff threshold is tested first, the small-cliff branch is effectively unreachable with the defaults.

Implement `Default` directly for `Config`, add `Config::load(path)` and `Config::validate()`, expose `--config`, and consider `#[serde(deny_unknown_fields)]` to catch misspellings.

### 6. Rendering modules combine policy, parsing, and graphics

Priority: high

Several modules are large:

- `pullautin_contours_render.rs`: 749 lines.
- `coastlines.rs`: 610 lines.
- `map_renderer.rs`: 554 lines.
- `vectors.rs`: 384 lines.

For example, [`src/vectors.rs`](../src/vectors.rs) simultaneously reads shapefiles, parses loosely typed OSM attributes, classifies ISOM symbols, performs coastline topology, and drives rendering.

Split vector processing into:

- `VectorFeature` parsing from shapefile records.
- A pure `classify(feature) -> Symbol` policy.
- Geometry preparation and clipping.
- Symbol rendering and layer composition.

Likewise, preserve the Pullautin implementation as a dedicated algorithm module, but extract coordinate conversion, raster access, and shapefile I/O from the numerical logic.

### 7. Memory and concurrency are not coordinated

Priority: high

`MapRenderer` holds many full-sized Skia surfaces simultaneously, while TIFF decoding explicitly uses unlimited limits and loads entire rasters. Batch mode then runs several renderers concurrently based only on a user-selected thread count.

Add conservative TIFF limits, checked dimension arithmetic, and a memory-aware worker limit. Longer term, compose short-lived layers sequentially or use tiled raster processing rather than retaining every layer at full resolution.

### 8. Networking and external-tool integration need stronger boundaries

Priority: high

Overpass requests are blocking and use a client without an explicit request timeout. Retries apply broadly to unsuccessful statuses, and an unbounded `Retry-After` value can cause unexpectedly long sleeps. External tool paths and versions are not preflighted.

Recommended changes:

- Set connect and total request timeouts.
- Retry only transient statuses such as 429 and selected 5xx responses.
- Use capped exponential backoff with jitter.
- Download into a `.part` file and rename only after validation.
- Add a startup preflight reporting PDAL, GDAL, PROJ, and `ogr2ogr` availability and versions.
- Pass `Path`/`OsStr` directly to commands instead of converting paths with `to_str().unwrap()`.

### 9. The public library API is difficult to use safely

Priority: medium

The crate exposes a library facade, but it accepts long positional argument lists, borrows `PathBuf` rather than `Path`, returns no result values, and does not expose the validated configuration or artifact types needed by integrators.

Prefer APIs such as:

```rust
pub fn process_tile(request: ProcessTileRequest) -> Result<RenderArtifacts, CassiniError>;
pub fn render_tile(request: RenderTileRequest) -> Result<RenderArtifacts, CassiniError>;
pub fn process_batch(request: BatchRequest) -> Result<BatchReport, CassiniError>;
```

This also gives future options a home without repeatedly breaking function signatures.

### 10. Test and CI coverage is too narrow

Priority: high

Only five unit tests exist: three retry parser tests and two contour geometry tests.

Missing coverage includes:

- Configuration parsing and validation.
- Empty and zero-thread batches.
- Tile discovery and neighbor calculation.
- Failed external commands.
- Stale artifact handling.
- OSM tag classification.
- Raster boundary calculations.
- World-file and georeferencing correctness.
- End-to-end output from a small fixture.

The current GitHub workflows only build releases. They do not run on pull requests or execute tests, formatting, or linting.

Add a normal CI workflow running:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
```

Then add a small integration fixture and golden-image tests with tolerance or perceptual comparison.

### 11. Documentation is manually duplicated and has drifted

Priority: medium

Comments in the CLI and configuration source instruct maintainers to update documentation manually. Drift is already visible: the CLI reference includes configuration values different from the implementation and reports version `0.4.0`, while the crate version is `0.16.0`.

Generate CLI reference output from Clap and generate the default configuration example from `Config::default()`. Test generated documentation in CI or keep generated fragments checked in with a drift check.

### 12. Build and release reproducibility can be improved

Priority: medium

Release workflows compile and publish but do not test. The Dockerfile uses an unpinned PDAL base and copies a prebuilt host artifact before installing a runtime GDAL version dynamically. This makes compatibility and reproducibility harder to reason about.

Use a multi-stage Docker build, pin the base image by version or digest, verify tool versions, run tests before release jobs, and add dependency vulnerability/license checks.

## Suggested target structure

```text
src/
  domain/
    extent.rs
    tile.rs
    config.rs
  pipeline/
    mod.rs
    artifacts.rs
    manifest.rs
    batch.rs
  adapters/
    pdal.rs
    gdal.rs
    overpass.rs
    process_runner.rs
  render/
    transform.rs
    vegetation.rs
    contours/
    cliffs.rs
    vectors/
    compositor.rs
  cli.rs
  lib.rs
  main.rs
```

This need not become a framework of traits. One `ProcessRunner` abstraction, typed artifacts, and pure domain functions would provide most of the testability benefit.

## Recommended implementation sequence

### Phase 1: correctness and failure handling

- Introduce `Result` throughout the public and stage APIs.
- Add checked subprocess execution and dependency preflight.
- Validate configuration, extents, inputs, and thread counts.
- Fix hard-coded DPI, stale undergrowth, empty batches, and zero-thread handling.
- Add regression tests for each fix.

### Phase 2: artifact safety and test seams

- Add versioned manifests and typed artifact paths.
- Use temporary working directories and atomic completion.
- Define cache validity and `--force`/resume behavior.
- Make subprocess execution replaceable by a fake in tests.

### Phase 3: automated quality gates

- Make formatting and Clippy clean.
- Add pull-request CI.
- Add a miniature end-to-end fixture and golden rendering checks.
- Add dependency security and license checks.

### Phase 4: structural decomposition

- Separate vector parsing, classification, geometry, and rendering.
- Centralize geographic-to-pixel transformations.
- Separate Pullautin numerical logic from I/O.
- Replace long positional APIs with request and result types.

### Phase 5: performance hardening

- Measure peak memory per stage and tile size.
- Bound TIFF and image dimensions.
- Replace static thread chunks with a bounded work queue.
- Reduce the number and lifetime of full-size rendering surfaces.

## Expected outcome

The first two phases would materially improve robustness without changing the core mapping algorithms. Failures would become attributable, incomplete results would no longer appear successful, reruns would be safer, and the pipeline would become testable without requiring every external geospatial executable. The later phases can then improve structure and performance with much lower regression risk.
