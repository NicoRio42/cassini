# Cassini render pipeline performance report

Date: 2026-09-08

## Executive summary

The `cassini render` pipeline has several worthwhile performance opportunities below the orchestration layer. This report intentionally excludes artifact caching and pipeline parallelization because those responsibilities are handled by the external orchestrator.

The recommended implementation order is:

1. Use a `Float32` buffered DEM and bypass VRT creation when there are no neighbors.
2. Remove the unbalanced Skia canvas `save()` calls.
3. Reduce the number of full-resolution Skia surfaces and avoid compositing empty layers.
4. Improve vegetation memory access and replace two-dimensional Gaussian sampling with separable passes.
5. Keep intermediate vegetation, contour, and cliff images in memory instead of encoding and decoding PNG files.
6. Simplify cliff and contour rasterization.
7. Stream vector input and final WebP output with fewer allocations and copies.

The first two items are relatively contained. Reducing layers and changing rasterization have larger potential benefits but require pixel-level regression tests.

## Scope and method

The review follows the single-tile `render` path from [`src/main.rs`](../../src/main.rs) through [`src/render.rs`](../../src/render.rs), including raster buffering, vegetation, DEM preparation, contours, cliffs, vectors, compositing, and lossless WebP encoding.

A representative 1 km tile was rendered at 600 DPI in a release build with merged undergrowth and vector rendering enabled. The filesystem cache was warm. The current working tree, including its in-progress lossless WebP output changes, was used.

| Stage | Wall time | Approximate share |
|---|---:|---:|
| Vegetation | 5.0 s | 23% |
| DEM buffering and hole filling | 10.6 s | 49% |
| Contour extraction and rendering | 1.7 s | 8% |
| Slope generation and cliff rendering | 2.3 s | 11% |
| OSM conversion, vector rendering, composition, and WebP | 2.0 s | 9% |
| **Total** | **21.6 s** | **100%** |

Peak resident memory was approximately 371 MB. These numbers are measurements from one representative tile, not a full benchmark suite; terrain complexity and vector density will change the balance.

All 13 existing tests passed with `cargo test --quiet`. The repository does not currently contain a render benchmark harness.

## Findings and recommendations

### 1. Materialize the buffered DEM as `Float32`

Priority: high

[`create_dem_with_buffer_and_slopes_tiff`](../../src/dem.rs) currently creates a buffered DEM through [`create_tif_with_buffer`](../../src/buffer.rs) without specifying an output type. The source DEM and buffered result are consequently `Float64`. `gdal_fillnodata` dominates the measured render and has to process the complete 2800 by 2800 raster.

An isolated comparison on the representative tile produced the following result:

| Buffered DEM | File size | Fill time | Fill peak RSS |
|---|---:|---:|---:|
| `Float64` | 60 MiB | 7.8 s | 293 MB |
| `Float32` | 30 MiB | 6.2 s | 201 MB |

Using `Float32` reduced fill time by approximately 20%, fill memory by 31%, and file size by 50%. `gdal_translate` can perform this conversion by adding `-ot Float32` while creating `dem_with_buffer.tif`.

`Float32` should provide more than enough precision for the elevations and contour intervals used by Cassini, but this must be established with regression tests rather than assumed. Compare at least:

- Raw contour count and elevations.
- Smoothed contour geometry within a documented tolerance.
- Cliff classification around both configured thresholds.
- Final image differences on flat, steep, and noisy terrain fixtures.

If acceptable, producing the DEM as `Float32` during LiDAR preprocessing would also reduce upstream storage and decoding costs. The render-side conversion can be implemented independently first.

### 2. Bypass `gdalbuildvrt` when there are no neighbors

Priority: high

[`create_tif_with_buffer`](../../src/buffer.rs) always launches `gdalbuildvrt`, even when its input list contains only the tile raster. A VRT is useful for mosaicking neighbors but adds no value for a single source.

On the test system, creating one single-source VRT took approximately 0.27 seconds. The render path does this for the three vegetation rasters and the DEM, making the avoidable overhead roughly 1.1 seconds per tile without neighbors.

When `neighbor_tiles.is_empty()`, pass the source TIFF directly to `gdal_translate`. Retain the VRT path when neighbors are present. Output equivalence should be checked with `gdalinfo` metadata and pixel comparison.

### 3. Remove unbalanced Skia canvas state saves

Priority: high

[`Canvas::draw_polyline`](../../src/canvas.rs), `draw_filled_polygon`, and `draw_filled_polygon_with_holes` call `Canvas::save()` after every draw. That method invokes Skia's canvas `save()` operation, but there is no corresponding `restore()` anywhere in the renderer.

Skia `save()` pushes matrix and clipping state; it does not flush or commit a drawing operation. The current behavior therefore grows the save stack once per drawn geometry without providing a rendering benefit. This can become significant on vector-dense tiles.

Remove these calls. If a future operation needs a temporary transform or clip, place a paired save/restore around that operation only. This change should be output-identical and is a good early optimization to validate with a pixel comparison.

### 4. Reduce eager full-resolution layers

Priority: high for memory, medium for single-tile latency

[`MapRenderer`](../../src/map_renderer.rs) eagerly owns approximately thirteen full-resolution RGBA Skia canvases. At 600 DPI, a 1 km tile is 2362 by 2362 pixels and each canvas occupies about 21.3 MiB before allocator and Skia overhead. The layer set therefore accounts for roughly 277 MiB by itself, consistent with the observed 371 MB process peak.

The renderer also overlays every layer during final composition, including layers that remained empty. This is especially wasteful with `--skip-vector`, which still allocates the vector canvases.

Possible implementation levels, from least to most invasive:

1. Lazily allocate optional layers and track whether each layer received any drawing commands. Skip absent or clean layers during composition.
2. Reuse canvases whose contents do not need to coexist.
3. Classify vector features into ordered display lists, then render each drawing-order group directly onto the final canvas. This preserves symbol ordering without retaining one bitmap per symbol category.

The third approach offers the largest memory reduction and removes most full-image overlay passes. It also changes renderer structure substantially, so drawing-order tests are essential. The existing final order in `MapRenderer::save_as_lossless_webp` should become an explicit, tested contract.

### 5. Improve vegetation traversal and filtering

Priority: high

The central vegetation loop in [`render_vegetation`](../../src/vegetation.rs) traverses `x` in the outer loop and `y` in the inner loop. TIFF pixels and `RgbaImage` pixels are row-major, so the current traversal repeatedly jumps by an entire row instead of accessing adjacent memory.

The loop also performs, per 1 m cell:

- A neighborhood minimum for high vegetation.
- A 5 by 5 Gaussian convolution for medium vegetation.
- Another 5 by 5 convolution in merged undergrowth mode, or a 9 by 9 convolution in the separate-symbol modes.
- Bounds checks and weight re-normalization for every sample, even though the 200-cell buffer makes all samples in the rendered interior valid.
- Individual rectangle or ellipse drawing calls for classified cells.

Recommended changes:

1. Process row-major row chunks. Because adjacent rectangles can overlap after scaling, preserve or explicitly define color precedence rather than merely swapping the existing loops and assuming identical output.
2. Flatten convolution kernels and image access to contiguous slices.
3. Remove interior bounds checks after validating dimensions once at the function boundary.
4. Implement the Gaussian as horizontal and vertical one-dimensional passes. The generated Gaussian kernel is separable, reducing a 5 by 5 filter from 25 samples to 10 and a 9 by 9 filter from 81 samples to 18.
5. In merged mode, sum medium and low vegetation into a sufficiently wide intermediate value before filtering. Linearity permits one convolution instead of two, subject to floating-point threshold tests.
6. Consider classifying on the logical 1 m grid and then rasterizing runs or upscaling, instead of issuing up to one drawing call per grid cell.

The separable implementation may change floating-point rounding close to thresholds. Golden images should include cells immediately below, equal to, and immediately above each vegetation threshold.

### 6. Avoid intermediate PNG round-trips

Priority: medium

Vegetation, contours, and cliffs are each encoded as full-resolution PNG files and then decoded into Skia canvases by `MapRenderer`:

- [`render_vegetation`](../../src/vegetation.rs)
- [`pullautin_cull_formlines_render_contours`](../../src/pullautin_contours_render.rs)
- [`render_cliffs`](../../src/cliffs.rs)
- [`MapRenderer::new`](../../src/map_renderer.rs)

Return in-memory image or surface values from these stages and pass them directly into the compositor. This removes three PNG encodes, writes, reads, and decodes. Intermediate PNG output can remain available behind an explicit diagnostic option if it is useful for debugging.

Choose one canonical pixel representation at the stage boundary. Repeated conversion between `image::RgbaImage`, Skia premultiplied surfaces, and unpremultiplied output can otherwise introduce both copies and subtle alpha differences.

### 7. Simplify cliff rasterization

Priority: medium to low

[`render_cliffs`](../../src/cliffs.rs) scans every buffered slope pixel and calculates output coordinates before checking whether the slope exceeds either cliff threshold. It also performs division and remainder for every element to recover `(x, y)`.

Improvements that should preserve behavior:

- Iterate over rows with `chunks_exact(slopes_width)` instead of computing division and remainder.
- Calculate the exact source index range corresponding to the output tile and skip the surrounding buffer wholesale.
- Test the slope threshold before converting coordinates.
- Precompute the two output ellipse radii.

The measured cliff renderer took about one second, so these are useful but less important than DEM and vegetation improvements.

### 8. Use stroked paths for contour thickness

Priority: medium to low

Contour rendering in [`pullautin_contours_render.rs`](../../src/pullautin_contours_render.rs) simulates line width by drawing each segment repeatedly at a grid of pixel offsets. Depending on contour class, this can issue approximately 16 to 49 line operations for one logical segment.

Render a contour polyline once with a configured Skia stroke width and dash pattern. Besides reducing draw calls, this would consolidate contour and vector rasterization onto the same graphics backend.

The output will not necessarily be pixel-identical because stroke joins, caps, antialiasing, and dash phase differ from the current offset-line algorithm. Treat this as a visual rendering change and approve it with representative golden images at multiple DPI settings.

### 9. Reduce vector conversion and parsing work

Priority: low for typical rural tiles; potentially higher for dense urban tiles

The render pipeline invokes `ogr2ogr` twice on the same OSM input, once for multipolygons and once for lines. It then loads each complete shapefile into memory in [`render_map_with_osm_vector_shapes`](../../src/vectors.rs). Every line also constructs a `HashMap<String, String>` for `other_tags`, including temporary vectors and owned strings.

Potential improvements:

- Convert all required OSM layers in one GDAL operation, preferably into a multi-layer format such as GeoPackage, or read the OSM layers directly.
- Stream shapefile records instead of collecting the complete layer.
- Parse only the `power`, `natural`, and `tunnel` keys needed from `other_tags`, without allocating a general-purpose map.
- Change `MapRenderer` drawing methods to take `&mut self` rather than consuming and returning the renderer for every feature. This makes the intended mutation explicit and avoids relying on compiler optimization of repeated aggregate moves.

Vector conversion and rendering were only about two seconds on the representative rural tile, but the current design scales with OSM density.

### 10. Reduce final WebP copies

Priority: low for latency, medium for memory

[`Canvas::lossless_webp_data`](../../src/canvas.rs) allocates a complete unpremultiplied RGBA pixel buffer, encodes into a second growing `Vec<u8>`, and only then writes that vector to a file.

Investigate the following, in order:

1. Encode directly to a `BufWriter<File>` rather than an intermediate encoded vector.
2. Access the raster surface pixels without an additional full-canvas copy when Skia's pixel layout and alpha representation are compatible with the encoder.
3. Benchmark alternative lossless WebP backends or effort settings only after the allocation changes.

Final vector rendering, composition, and WebP output together accounted for about 1.2 seconds after OSM conversion in the measured run, so codec tuning should not displace the higher-priority work.

## Benchmark and validation plan

Before changing the larger algorithms, add a stable performance and visual baseline:

- At least one rural, one urban/vector-dense, and one steep/contour-dense tile.
- Release builds only, with separate cold- and warm-filesystem results when I/O is relevant.
- Stage timings and peak RSS, recorded in a machine-readable format.
- Pixel-golden tests for changes expected to be exact.
- Difference images and explicit tolerances for `Float32`, separable convolution, and Skia contour strokes.
- Measurements at the default DPI and at least one non-default DPI.
- `--skip-vector` and each undergrowth mode represented.

Microbenchmarks are especially appropriate for vegetation filters, cliff classification, `other_tags` parsing, canvas composition, and WebP encoding. End-to-end measurements remain necessary because GDAL subprocess and memory behavior dominate parts of the pipeline.

## Portability issue found during profiling

The unmodified render command did not run against the local GDAL 3.8 installation:

- The code passes `--quiet` to GDAL utilities, while these programs accept `-q` on this installation.
- The installation provides `gdal_fillnodata.py`, not an executable named `gdal_fillnodata`.

Temporary wrappers were used only for the benchmark. Prefer the portable `-q` form and either discover the fill-nodata executable at startup or call the GDAL API directly. This is primarily an operability fix, but reliable local benchmarking depends on it.

## Proposed implementation batches

### Batch 1: contained, output-preserving changes

- Remove unbalanced canvas `save()` calls.
- Bypass VRT creation without neighbors.
- Simplify the cliff scan.
- Stream encoded WebP directly to a buffered file.
- Add benchmarks and exact pixel comparisons.

### Batch 2: numeric representation and vegetation

- Convert the buffered DEM to `Float32`.
- Make vegetation traversal cache-friendly.
- Implement separable vegetation filters.
- Validate numeric and pixel differences around thresholds.

### Batch 3: rendering architecture

- Pass intermediate layers in memory.
- Allocate vector layers lazily as an interim improvement.
- Replace bitmap-per-symbol layers with ordered display lists or reusable canvases.
- Move contour drawing to stroked Skia paths if the visual result is accepted.

These batches deliberately contain no artifact caching or pipeline parallelization work.
