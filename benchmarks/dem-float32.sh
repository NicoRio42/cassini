#!/usr/bin/env bash

set -eo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
    echo "usage: $0 INPUT_LAZ '([MIN_X,MAX_X],[MIN_Y,MAX_Y])' [WORK_DIR]" >&2
    exit 2
fi

input_laz=$1
crop_bounds=$2
work_dir=${3:-target/dem-float32-benchmark}
repetitions=${CASSINI_BENCHMARK_REPETITIONS:-5}
conda_environment=${CASSINI_CONDA_ENV:-cassini}

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

if ! command -v conda >/dev/null 2>&1; then
    echo "conda is required" >&2
    exit 1
fi

# `conda activate` is a shell function, so initialize it explicitly for scripts.
source "$(conda info --base)/etc/profile.d/conda.sh"
conda activate "$conda_environment"
set -u

for program in pdal gdalbuildvrt gdal_translate gdal_fillnodata gdal_contour gdaldem; do
    if ! command -v "$program" >/dev/null 2>&1; then
        echo "$program is required in the '$conda_environment' environment" >&2
        exit 1
    fi
done

mkdir -p "$work_dir"
subset_laz="$work_dir/subset.laz"
lidar_dir="$work_dir/lidar"

if [[ ! -f "$subset_laz" ]]; then
    pdal translate "$input_laz" "$subset_laz" crop \
        --filters.crop.bounds="$crop_bounds" \
        --writers.las.forward=all
fi

if [[ ! -f "$lidar_dir/extent.txt" ]]; then
    cargo run --release -- lidar "$subset_laz" --output-dir "$lidar_dir"
fi

extent=$(<"$lidar_dir/extent.txt")
IFS='|' read -r min_x min_y max_x max_y <<< "$extent"
buffer=200
upper_left_x=$((min_x - buffer))
upper_left_y=$((max_y + buffer))
lower_right_x=$((max_x + buffer))
lower_right_y=$((min_y - buffer))

vrt_path="$work_dir/dem.vrt"
gdalbuildvrt -q "$vrt_path" "$lidar_dir/dem.tif"

results_path="$work_dir/results.csv"
printf 'raster_type,iteration,fill_seconds,fill_peak_kib\n' > "$results_path"

for raster_type in Float64 Float32; do
    output_path="$work_dir/dem_${raster_type}.tif"

    for ((iteration = 1; iteration <= repetitions; iteration++)); do
        gdal_translate -q \
            -projwin "$upper_left_x" "$upper_left_y" "$lower_right_x" "$lower_right_y" \
            -of GTiff -tr 0.5 0.5 -ot "$raster_type" \
            "$vrt_path" "$output_path"

        /usr/bin/time -a -o "$results_path" \
            -f "$raster_type,$iteration,%e,%M" \
            gdal_fillnodata "$output_path" "$output_path" >/dev/null
    done

    gdal_translate -q -tr 2 2 -r average \
        "$output_path" "$work_dir/dem_2m_${raster_type}.tif"
    gdal_contour -q -a elev -i 2.5 -f GPKG \
        "$work_dir/dem_2m_${raster_type}.tif" \
        "$work_dir/contours_${raster_type}.gpkg"
    gdaldem slope -q \
        "$output_path" "$work_dir/slopes_${raster_type}.tif"
done

python benchmarks/validate-dem-float32.py "$work_dir" \
    > "$work_dir/validation.json"

echo "Benchmark results: $results_path"
echo "Validation results: $work_dir/validation.json"
awk -F, '
    NR > 1 { seconds[$1] += $3; rss[$1] += $4; count[$1]++ }
    END {
        for (type in count) {
            printf "%s mean fill: %.3f s; mean peak RSS: %.1f MiB\n", \
                type, seconds[type] / count[type], rss[type] / count[type] / 1024
        }
    }
' "$results_path"
du -h "$work_dir/dem_Float64.tif" "$work_dir/dem_Float32.tif"
