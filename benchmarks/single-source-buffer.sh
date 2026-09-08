#!/usr/bin/env bash

set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 LIDAR_DIR [WORK_DIR]" >&2
    exit 2
fi

lidar_dir=$1
work_dir=${2:-target/single-source-buffer-benchmark}
repetitions=${CASSINI_BENCHMARK_REPETITIONS:-10}

if [[ ! "$repetitions" =~ ^[1-9][0-9]*$ ]]; then
    echo "CASSINI_BENCHMARK_REPETITIONS must be a positive integer" >&2
    exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

for program in gdalbuildvrt gdal_translate gdalcompare.py; do
    if ! command -v "$program" >/dev/null 2>&1; then
        echo "$program is required" >&2
        exit 1
    fi
done

extent_path="$lidar_dir/extent.txt"
if [[ ! -f "$extent_path" ]]; then
    echo "missing extent file: $extent_path" >&2
    exit 1
fi

for raster_name in dem high_vegetation medium_vegetation low_vegetation; do
    if [[ ! -f "$lidar_dir/$raster_name.tif" ]]; then
        echo "missing source raster: $lidar_dir/$raster_name.tif" >&2
        exit 1
    fi
done

extent=$(<"$extent_path")
IFS='|' read -r min_x min_y max_x max_y <<< "$extent"
buffer=200
upper_left_x=$((min_x - buffer))
upper_left_y=$((max_y + buffer))
lower_right_x=$((max_x + buffer))
lower_right_y=$((min_y - buffer))

mkdir -p "$work_dir/vrt" "$work_dir/direct"
results_path="$work_dir/results.csv"
printf 'strategy,iteration,total_seconds\n' > "$results_path"

run_strategy() {
    local strategy=$1
    local iteration=$2
    local start_ns end_ns elapsed_seconds

    start_ns=$(date +%s%N)
    for raster_name in dem high_vegetation medium_vegetation low_vegetation; do
        local resolution=1
        local input_path="$lidar_dir/$raster_name.tif"
        local output_path="$work_dir/$strategy/${raster_name}_with_buffer.tif"
        local translate_arguments=(
            -q
            -projwin "$upper_left_x" "$upper_left_y" "$lower_right_x" "$lower_right_y"
            -of GTiff
        )

        if [[ "$raster_name" == dem ]]; then
            resolution=0.5
            translate_arguments+=(-ot Float32)
        fi
        translate_arguments+=(-tr "$resolution" "$resolution")

        if [[ "$strategy" == vrt ]]; then
            local vrt_path="$work_dir/vrt/${raster_name}_with_buffer.vrt"
            gdalbuildvrt -q "$vrt_path" "$input_path"
            input_path=$vrt_path
        fi

        gdal_translate "${translate_arguments[@]}" "$input_path" "$output_path"
    done
    end_ns=$(date +%s%N)
    elapsed_seconds=$(awk -v start="$start_ns" -v end="$end_ns" \
        'BEGIN { printf "%.6f", (end - start) / 1000000000 }')
    printf '%s,%s,%s\n' "$strategy" "$iteration" "$elapsed_seconds" >> "$results_path"
}

# Alternate the order to reduce cache and thermal bias between the strategies.
for ((iteration = 1; iteration <= repetitions; iteration++)); do
    if ((iteration % 2 == 1)); then
        run_strategy vrt "$iteration"
        run_strategy direct "$iteration"
    else
        run_strategy direct "$iteration"
        run_strategy vrt "$iteration"
    fi
done

for raster_name in dem high_vegetation medium_vegetation low_vegetation; do
    gdalcompare.py -skip_binary \
        "$work_dir/vrt/${raster_name}_with_buffer.tif" \
        "$work_dir/direct/${raster_name}_with_buffer.tif"
done

echo "Benchmark results: $results_path"
awk -F, '
    NR > 1 { seconds[$1] += $3; count[$1]++ }
    END {
        vrt_mean = seconds["vrt"] / count["vrt"]
        direct_mean = seconds["direct"] / count["direct"]
        printf "VRT mean: %.3f s; direct mean: %.3f s; saved: %.3f s (%.1f%%)\n", \
            vrt_mean, direct_mean, vrt_mean - direct_mean, \
            100 * (vrt_mean - direct_mean) / vrt_mean
    }
' "$results_path"
