#!/usr/bin/env python3

import argparse
import json
from pathlib import Path

import numpy as np
from osgeo import gdal, ogr

gdal.UseExceptions()


def read_raster(path: Path) -> tuple[np.ndarray, float | None]:
    dataset = gdal.Open(str(path), gdal.GA_ReadOnly)
    if dataset is None:
        raise RuntimeError(f"could not open {path}")
    band = dataset.GetRasterBand(1)
    return band.ReadAsArray(), band.GetNoDataValue()


def valid_mask(array: np.ndarray, nodata: float | None) -> np.ndarray:
    mask = np.isfinite(array)
    if nodata is not None:
        mask &= array != nodata
    return mask


def raster_difference(float64_path: Path, float32_path: Path) -> dict[str, float | int]:
    float64, float64_nodata = read_raster(float64_path)
    float32, float32_nodata = read_raster(float32_path)
    if float64.shape != float32.shape:
        raise RuntimeError(f"raster dimensions differ: {float64.shape} != {float32.shape}")

    float64_valid = valid_mask(float64, float64_nodata)
    float32_valid = valid_mask(float32, float32_nodata)
    shared_valid = float64_valid & float32_valid
    validity_mismatches = int(np.count_nonzero(float64_valid != float32_valid))
    differences = np.abs(float64[shared_valid].astype(np.float64) - float32[shared_valid])

    return {
        "valid_pixels": int(np.count_nonzero(shared_valid)),
        "validity_mismatches": validity_mismatches,
        "different_pixels": int(np.count_nonzero(differences)),
        "maximum_absolute_difference": float(np.max(differences, initial=0.0)),
        "mean_absolute_difference": float(np.mean(differences)) if differences.size else 0.0,
    }


def read_contours(path: Path) -> list[tuple[float, ogr.Geometry]]:
    dataset = ogr.Open(str(path), 0)
    if dataset is None:
        raise RuntimeError(f"could not open {path}")
    layer = dataset.GetLayer(0)
    contours = []
    for feature in layer:
        contours.append((feature.GetFieldAsDouble("elev"), feature.GetGeometryRef().Clone()))
    return contours


def geometry_vertices(geometry: ogr.Geometry) -> list[tuple[float, float]]:
    if geometry.GetGeometryCount() > 0:
        return [
            point
            for index in range(geometry.GetGeometryCount())
            for point in geometry_vertices(geometry.GetGeometryRef(index))
        ]
    return [geometry.GetPoint_2D(index) for index in range(geometry.GetPointCount())]


def maximum_vertex_to_geometry_distance(
    first: ogr.Geometry, second: ogr.Geometry
) -> float:
    maximum = 0.0
    point = ogr.Geometry(ogr.wkbPoint)
    for x, y in geometry_vertices(first):
        point.Empty()
        point.AddPoint_2D(x, y)
        maximum = max(maximum, point.Distance(second))
    return maximum


def symmetric_vertex_distance(first: ogr.Geometry, second: ogr.Geometry) -> float:
    return max(
        maximum_vertex_to_geometry_distance(first, second),
        maximum_vertex_to_geometry_distance(second, first),
    )


def contour_difference(float64_path: Path, float32_path: Path) -> dict[str, object]:
    float64 = read_contours(float64_path)
    float32 = read_contours(float32_path)
    result: dict[str, object] = {
        "float64_count": len(float64),
        "float32_count": len(float32),
        "elevations_equal": [item[0] for item in float64] == [item[0] for item in float32],
    }

    if len(float64) == len(float32):
        distances = [
            symmetric_vertex_distance(first_geometry, second_geometry)
            for (_, first_geometry), (_, second_geometry) in zip(float64, float32)
        ]
        result["maximum_symmetric_vertex_distance"] = max(distances, default=0.0)
        result["mean_symmetric_vertex_distance"] = (
            float(np.mean(distances)) if distances else 0.0
        )
    return result


def cliff_classification_difference(
    float64_path: Path, float32_path: Path, thresholds: tuple[float, float]
) -> dict[str, object]:
    float64, float64_nodata = read_raster(float64_path)
    float32, float32_nodata = read_raster(float32_path)
    shared_valid = valid_mask(float64, float64_nodata) & valid_mask(float32, float32_nodata)

    def classify(slopes: np.ndarray) -> np.ndarray:
        classifications = np.zeros(slopes.shape, dtype=np.uint8)
        classifications[slopes > thresholds[0]] = 1
        classifications[slopes > thresholds[1]] = 2
        return classifications

    float64_classes = classify(float64)
    float32_classes = classify(float32)
    return {
        "thresholds": list(thresholds),
        "classification_mismatches": int(
            np.count_nonzero(float64_classes[shared_valid] != float32_classes[shared_valid])
        ),
        "float64_class_counts": np.bincount(
            float64_classes[shared_valid], minlength=3
        ).tolist(),
        "float32_class_counts": np.bincount(
            float32_classes[shared_valid], minlength=3
        ).tolist(),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("work_dir", type=Path)
    parser.add_argument("--cliff-threshold-1", type=float, default=60.0)
    parser.add_argument("--cliff-threshold-2", type=float, default=60.0)
    arguments = parser.parse_args()
    work_dir = arguments.work_dir

    report = {
        "dem": raster_difference(
            work_dir / "dem_Float64.tif", work_dir / "dem_Float32.tif"
        ),
        "downsampled_dem": raster_difference(
            work_dir / "dem_2m_Float64.tif", work_dir / "dem_2m_Float32.tif"
        ),
        "contours": contour_difference(
            work_dir / "contours_Float64.gpkg",
            work_dir / "contours_Float32.gpkg",
        ),
        "slopes": raster_difference(
            work_dir / "slopes_Float64.tif", work_dir / "slopes_Float32.tif"
        ),
        "cliffs": cliff_classification_difference(
            work_dir / "slopes_Float64.tif",
            work_dir / "slopes_Float32.tif",
            (arguments.cliff_threshold_1, arguments.cliff_threshold_2),
        ),
    }
    checks = {
        "dem_validity_is_identical": report["dem"]["validity_mismatches"] == 0,
        "dem_difference_is_at_most_1mm": (
            report["dem"]["maximum_absolute_difference"] <= 0.001
        ),
        "downsampled_dem_difference_is_at_most_1mm": (
            report["downsampled_dem"]["maximum_absolute_difference"] <= 0.001
        ),
        "raw_contour_count_is_identical": (
            report["contours"]["float64_count"]
            == report["contours"]["float32_count"]
        ),
        "raw_contour_elevations_are_identical": report["contours"]["elevations_equal"],
        "raw_contour_displacement_is_at_most_2cm": (
            report["contours"].get("maximum_symmetric_vertex_distance", float("inf"))
            <= 0.02
        ),
        "slope_validity_is_identical": report["slopes"]["validity_mismatches"] == 0,
        "slope_difference_is_at_most_0.001_degrees": (
            report["slopes"]["maximum_absolute_difference"] <= 0.001
        ),
        "cliff_classification_is_identical": (
            report["cliffs"]["classification_mismatches"] == 0
        ),
    }
    report["acceptance"] = {"checks": checks, "passed": all(checks.values())}
    print(json.dumps(report, indent=2, sort_keys=True))

    if not report["acceptance"]["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
