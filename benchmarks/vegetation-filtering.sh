#!/usr/bin/env bash

set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

repetitions=${CASSINI_BENCHMARK_REPETITIONS:-5}
size=${CASSINI_VEGETATION_BENCHMARK_SIZE:-1000}

if [[ ! "$repetitions" =~ ^[1-9][0-9]*$ ]]; then
    echo "CASSINI_BENCHMARK_REPETITIONS must be a positive integer" >&2
    exit 2
fi
if [[ ! "$size" =~ ^[1-9][0-9]*$ ]]; then
    echo "CASSINI_VEGETATION_BENCHMARK_SIZE must be a positive integer" >&2
    exit 2
fi

CASSINI_BENCHMARK_REPETITIONS=$repetitions \
    CASSINI_VEGETATION_BENCHMARK_SIZE=$size \
    cargo test --release vegetation::tests::benchmark_vegetation_classification -- \
        --ignored --nocapture --test-threads=1
