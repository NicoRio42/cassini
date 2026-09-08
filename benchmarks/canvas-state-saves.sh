#!/usr/bin/env bash

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

repetitions=${CASSINI_BENCHMARK_REPETITIONS:-10}
geometries=${CASSINI_CANVAS_BENCHMARK_GEOMETRIES:-100000}

if [[ ! "$repetitions" =~ ^[1-9][0-9]*$ ]]; then
    echo "CASSINI_BENCHMARK_REPETITIONS must be a positive integer" >&2
    exit 2
fi
if [[ ! "$geometries" =~ ^[1-9][0-9]*$ ]]; then
    echo "CASSINI_CANVAS_BENCHMARK_GEOMETRIES must be a positive integer" >&2
    exit 2
fi

CASSINI_BENCHMARK_REPETITIONS=$repetitions \
    CASSINI_CANVAS_BENCHMARK_GEOMETRIES=$geometries \
    cargo test --release canvas::tests::benchmark_vector_dense_canvas_drawing -- \
        --ignored --nocapture --test-threads=1
