# DEM `Float32` benchmark

`dem-float32.sh` isolates the buffered-DEM representation change described in
the first finding of `specs/research/render-pipeline-performance-report.md`.
It crops a real LAZ/COPC source with PDAL, runs Cassini's LiDAR preprocessing,
and measures otherwise-identical `Float64` and `Float32` buffering and
`gdal_fillnodata` runs.

The script expects the Conda environment in `CASSINI_CONDA_ENV` (default:
`cassini`) to provide PDAL, GDAL, NumPy, and the GDAL Python bindings. It writes
all generated data below `target/` by default, which is ignored by Git.

For the local Arselle data, run:

```sh
CASSINI_BENCHMARK_REPETITIONS=5 benchmarks/dem-float32.sh \
  in-arselle/LHD_FXX_0926_6450_PTS_LAMB93_IGN69.copc.laz \
  '([926350,926650],[6449350,6449650])'
```

The crop is 300 m by 300 m and contains about 2.3 million points. Pass a third
argument to select a different working directory.

The benchmark produces:

- `results.csv`: fill wall time and peak RSS for every repetition.
- `validation.json`: DEM, downsampled DEM, raw contour, slope, and cliff
  comparisons.
- Both buffered DEMs and derived artifacts for further inspection.

## Regression tolerances

The automated comparison requires identical valid-data masks, raw contour
counts and elevations, and cliff classifications at both configured
thresholds. It allows at most 1 mm elevation error, 0.001 degrees of slope
error, and 2 cm of raw-contour displacement.

For the full Cassini pipeline, smoothed contour displacement must remain below
20 cm and changed final-image pixels below 0.01% at 600 DPI. These latter two
checks require a render from the pre-change revision and are therefore recorded
manually rather than enforced by the single-revision benchmark script.

## Single-source buffering benchmark

`single-source-buffer.sh` measures the no-neighbor optimization described in
finding two of `specs/research/render-pipeline-performance-report.md`. It times
the four buffering operations performed during a render in two forms:

- The previous path, which creates a one-source VRT before each translation.
- The optimized path, which passes each source TIFF directly to
  `gdal_translate`.

Pass a Cassini LiDAR artifact directory containing `extent.txt`, `dem.tif`, and
the three vegetation TIFFs:

```sh
CASSINI_BENCHMARK_REPETITIONS=20 benchmarks/single-source-buffer.sh \
  target/dem-float32-benchmark/lidar
```

The strategies are alternated to reduce ordering bias. The script writes raw
timings to `target/single-source-buffer-benchmark/results.csv`, reports the mean
time and savings, and uses `gdalcompare.py` to require equivalent geospatial
metadata and pixel values for all four outputs.
