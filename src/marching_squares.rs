//! Contour lines of a raster, as `gdal_contour -i <interval>` extracts them.
//!
//! Ported from GDAL's marching squares (`alg/marching_squares/square.h`,
//! `level_generator.h`, `segment_merger.h`, `contour_generator.h`) and
//! `alg/contour.cpp` (GDAL 3.12), for lines only (no polygons):
//!
//! > Copyright (c) 2003, Frank Warmerdam <warmerdam@pobox.com>
//! > Copyright (c) 2003, Applied Coherent Technology Corporation, www.actgate.com
//! > Copyright (c) 2007-2013, Even Rouault <even dot rouault at spatialys.com>
//! > Copyright (c) 2018, Oslandia <infos at oslandia dot com>
//! >
//! > SPDX-License-Identifier: MIT
//!
//! The values are at cell centres. The grid is surrounded by nodata, and a
//! square with nodata corners is split into four sub-squares around its data
//! corners, so lines reach half a cell into nodata areas and past the outer
//! cell centres, up to the grid edge. Lines are joined from segments row by
//! row, and emitted, in the same order and with the same vertices as GDAL's,
//! as soon as a row adds nothing to them.

use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::Write,
    ops::Range,
    path::Path,
};

use shapefile::{
    dbase::{FieldIOError, FieldName, FieldWriter, TableWriterBuilder, WritableRecord},
    Point as ShapefilePoint, Polyline,
};

use crate::{
    error::{Result, ResultContext},
    grid::Grid,
};

/// The `.prj` OGR writes for EPSG:2154.
const LAMBERT_93_ESRI_WKT: &str = r#"PROJCS["RGF_1993_Lambert_93",GEOGCS["GCS_RGF_1993",DATUM["D_RGF_1993",SPHEROID["GRS_1980",6378137.0,298.257222101]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]],PROJECTION["Lambert_Conformal_Conic"],PARAMETER["False_Easting",700000.0],PARAMETER["False_Northing",6600000.0],PARAMETER["Central_Meridian",3.0],PARAMETER["Standard_Parallel_1",49.0],PARAMETER["Standard_Parallel_2",44.0],PARAMETER["Latitude_Of_Origin",46.5],UNIT["Meter",1.0]]"#;

/// A contour line, in projected coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct ContourLine {
    pub elevation: f64,
    pub points: Vec<(f64, f64)>,
}

/// The contour lines of `grid` at every multiple of `interval`, in the order
/// `gdal_contour` writes them.
pub fn contour_lines(grid: &Grid<f32>, interval: f64) -> Vec<ContourLine> {
    let values: Vec<f64> = grid
        .values
        .iter()
        .map(|&value| {
            if grid.has_data(value) {
                value as f64
            } else {
                f64::NAN
            }
        })
        .collect();
    let Some(levels) = levels(&values, interval) else {
        return Vec::new();
    };

    let mut merger = SegmentMerger::default();
    let nan_row = vec![f64::NAN; grid.width];
    let row = |index: usize| match index {
        0 => &nan_row[..],
        _ if index > grid.height => &nan_row[..],
        _ => &values[(index - 1) * grid.width..index * grid.width],
    };
    // Squares between the centres of rows `line - 1` and `line`, and columns
    // `col - 1` and `col`, both shifted by one so that row and column 0 are
    // the nodata outside the grid.
    for line in 0..=grid.height {
        merger.beginning_of_line();
        let (previous, current) = (row(line), row(line + 1));
        let value = |row: &[f64], col: usize| match col {
            0 => f64::NAN,
            _ => row.get(col - 1).copied().unwrap_or(f64::NAN),
        };
        for col in 0..=grid.width {
            let (left, right) = (col as f64 - 0.5, col as f64 + 0.5);
            let (upper, lower) = (line as f64 - 0.5, line as f64 + 0.5);
            Square::new(
                ValuedPoint::new(left, upper, value(previous, col)),
                ValuedPoint::new(right, upper, value(previous, col + 1)),
                ValuedPoint::new(left, lower, value(current, col)),
                ValuedPoint::new(right, lower, value(current, col + 1)),
                false,
            )
            .process(&levels, &mut merger);
        }
        merger.end_of_line();
    }
    merger.finish();

    // From cell coordinates to projected ones, as GDAL applies the geo
    // transform.
    merger
        .lines
        .into_iter()
        .map(|(level, points)| ContourLine {
            elevation: levels[level],
            points: points
                .into_iter()
                .map(|point| {
                    (
                        grid.min_x + grid.resolution * point.x,
                        grid.max_y + -grid.resolution * point.y,
                    )
                })
                .collect(),
        })
        .collect()
}

/// Writes `lines` to the shapefile at `path`, as `gdal_contour -a elev`
/// does: a `LineString` layer with fields `ID` (from 0) and `elev`, and an
/// EPSG:2154 `.prj`.
pub fn write_contour_shapefile(path: &Path, lines: &[ContourLine]) -> Result<()> {
    let context = || format!("could not write contour shapefile `{}`", path.display());
    let table = TableWriterBuilder::new()
        .add_numeric_field(FieldName::try_from("ID").expect("valid field name"), 8, 0)
        .add_numeric_field(
            FieldName::try_from("elev").expect("valid field name"),
            24,
            15,
        );
    let mut writer = shapefile::Writer::from_path(path, table).context(context())?;
    for (id, line) in lines.iter().enumerate() {
        let points = line
            .points
            .iter()
            .map(|&(x, y)| ShapefilePoint::new(x, y))
            .collect();
        let record = ContourRecord {
            id: id as f64,
            elevation: line.elevation,
        };
        writer
            .write_shape_and_record(&Polyline::new(points), &record)
            .context(context())?;
    }
    drop(writer);
    let prj_path = path.with_extension("prj");
    fs::write(&prj_path, LAMBERT_93_ESRI_WKT).context(format!(
        "could not write projection file `{}`",
        prj_path.display()
    ))
}

struct ContourRecord {
    id: f64,
    elevation: f64,
}

impl WritableRecord for ContourRecord {
    fn write_using<W: Write>(
        &self,
        field_writer: &mut FieldWriter<'_, W>,
    ) -> std::result::Result<(), FieldIOError> {
        field_writer.write_next_field_value(&self.id)?;
        field_writer.write_next_field_value(&self.elevation)
    }
}

/// The contour levels: the multiples of `interval` between the minimum and
/// maximum of `values`, ignoring NaN, or `None` if there are no levels.
fn levels(values: &[f64], interval: f64) -> Option<Vec<f64>> {
    let (min, max) =
        values
            .iter()
            .filter(|value| !value.is_nan())
            .fold(None, |range, &value| match range {
                None => Some((value, value)),
                Some((min, max)) => Some((f64::min(min, value), f64::max(max, value))),
            })?;
    if min == max {
        return None;
    }
    let level = |index: f64| index * interval;
    let mut first = (min / interval).ceil();
    let fudged_min = fudge(min, level(first));
    if fudged_min > min {
        first = (fudged_min / interval).ceil();
    }
    let mut end = (max / interval).floor() + 1.0;
    let fudged_max = fudge(max, level(end));
    if fudged_max > max {
        end = (fudged_max / interval).floor() + 1.0;
    }
    let levels: Vec<f64> = (first as i64..end as i64)
        .map(|index| level(index as f64))
        .collect();
    (!levels.is_empty()).then_some(levels)
}

/// The indices of the levels a square with values from `min` to `max`
/// crosses.
fn level_range(levels: &[f64], min: f64, max: f64) -> Range<usize> {
    let (min, max) = if min > max { (max, min) } else { (min, max) };
    let first = levels
        .iter()
        .position(|&level| level >= fudge(min, level))
        .unwrap_or(levels.len());
    if min == max {
        return first..first;
    }
    let end = levels[first..]
        .iter()
        .position(|&level| level > fudge(max, level))
        .map_or(levels.len(), |offset| first + offset);
    first..end
}

/// `value`, moved off `level` if it is too close to it.
fn fudge(value: f64, level: f64) -> f64 {
    const TOLERANCE: f64 = 1e-6;
    if (level - value).abs() < TOLERANCE {
        value + TOLERANCE
    } else {
        value
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Copy)]
struct ValuedPoint {
    x: f64,
    y: f64,
    value: f64,
}

impl ValuedPoint {
    fn new(x: f64, y: f64, value: f64) -> Self {
        Self { x, y, value }
    }
}

#[derive(Clone, Copy)]
enum Border {
    Left,
    Lower,
    Right,
    Upper,
}

/// The bits of the marching squares cases: which corners are above the
/// level.
const UPPER_LEFT: u8 = 1;
const LOWER_LEFT: u8 = 2;
const LOWER_RIGHT: u8 = 4;
const UPPER_RIGHT: u8 = 8;
const ALL_LOW: u8 = 0;
const ALL_HIGH: u8 = UPPER_LEFT | LOWER_LEFT | LOWER_RIGHT | UPPER_RIGHT;
const UPPER_LEFT_LOWER_LEFT: u8 = UPPER_LEFT | LOWER_LEFT;
const LOWER_LEFT_LOWER_RIGHT: u8 = LOWER_LEFT | LOWER_RIGHT;
const LOWER_RIGHT_UPPER_RIGHT: u8 = LOWER_RIGHT | UPPER_RIGHT;
const UPPER_RIGHT_UPPER_LEFT: u8 = UPPER_RIGHT | UPPER_LEFT;
const ALL_BUT_UPPER_LEFT: u8 = ALL_HIGH & !UPPER_LEFT;
const ALL_BUT_LOWER_LEFT: u8 = ALL_HIGH & !LOWER_LEFT;
const ALL_BUT_LOWER_RIGHT: u8 = ALL_HIGH & !LOWER_RIGHT;
const ALL_BUT_UPPER_RIGHT: u8 = ALL_HIGH & !UPPER_RIGHT;

struct Square {
    upper_left: ValuedPoint,
    lower_left: ValuedPoint,
    lower_right: ValuedPoint,
    upper_right: ValuedPoint,
    nan_count: usize,
    /// Whether this is a sub-square of a square with nodata corners.
    split: bool,
}

impl Square {
    fn new(
        upper_left: ValuedPoint,
        upper_right: ValuedPoint,
        lower_left: ValuedPoint,
        lower_right: ValuedPoint,
        split: bool,
    ) -> Self {
        let nan_count = [upper_left, upper_right, lower_left, lower_right]
            .iter()
            .filter(|point| point.value.is_nan())
            .count();
        Self {
            upper_left,
            lower_left,
            lower_right,
            upper_right,
            nan_count,
            split,
        }
    }

    fn process(&self, levels: &[f64], merger: &mut SegmentMerger) {
        if self.nan_count == 4 {
            return;
        }
        if self.nan_count > 0 {
            if !self.upper_left.value.is_nan() {
                self.upper_left_square().process(levels, merger);
            }
            if !self.upper_right.value.is_nan() {
                self.upper_right_square().process(levels, merger);
            }
            if !self.lower_left.value.is_nan() {
                self.lower_left_square().process(levels, merger);
            }
            if !self.lower_right.value.is_nan() {
                self.lower_right_square().process(levels, merger);
            }
            return;
        }

        let corners = [
            self.upper_left,
            self.upper_right,
            self.lower_left,
            self.lower_right,
        ];
        let min = corners
            .iter()
            .map(|point| point.value)
            .fold(f64::INFINITY, f64::min);
        let max = corners
            .iter()
            .map(|point| point.value)
            .fold(f64::NEG_INFINITY, f64::max);
        for level in level_range(levels, min, max) {
            for (start, end) in self.segments(levels[level]) {
                merger.add_segment(level, start, end);
            }
        }
    }

    fn upper_left_square(&self) -> Self {
        Self::new(
            self.upper_left,
            self.upper_center(),
            self.left_center(),
            self.center(),
            true,
        )
    }

    fn lower_left_square(&self) -> Self {
        Self::new(
            self.left_center(),
            self.center(),
            self.lower_left,
            self.lower_center(),
            true,
        )
    }

    fn lower_right_square(&self) -> Self {
        Self::new(
            self.center(),
            self.right_center(),
            self.lower_center(),
            self.lower_right,
            true,
        )
    }

    fn upper_right_square(&self) -> Self {
        Self::new(
            self.upper_center(),
            self.upper_right,
            self.center(),
            self.right_center(),
            true,
        )
    }

    /// The mean of the data corners.
    fn center(&self) -> ValuedPoint {
        let or_zero = |value: f64| if value.is_nan() { 0.0 } else { value };
        ValuedPoint::new(
            0.5 * (self.upper_left.x + self.lower_right.x),
            0.5 * (self.upper_left.y + self.lower_right.y),
            (or_zero(self.lower_left.value)
                + or_zero(self.upper_left.value)
                + or_zero(self.lower_right.value)
                + or_zero(self.upper_right.value))
                / (4 - self.nan_count) as f64,
        )
    }

    fn left_center(&self) -> ValuedPoint {
        ValuedPoint::new(
            self.upper_left.x,
            0.5 * (self.upper_left.y + self.lower_left.y),
            mean_of_data(self.upper_left.value, self.lower_left.value),
        )
    }

    fn lower_center(&self) -> ValuedPoint {
        ValuedPoint::new(
            0.5 * (self.lower_left.x + self.lower_right.x),
            self.lower_left.y,
            mean_of_data(self.lower_left.value, self.lower_right.value),
        )
    }

    fn right_center(&self) -> ValuedPoint {
        ValuedPoint::new(
            self.upper_right.x,
            0.5 * (self.upper_right.y + self.lower_right.y),
            mean_of_data(self.upper_right.value, self.lower_right.value),
        )
    }

    fn upper_center(&self) -> ValuedPoint {
        ValuedPoint::new(
            0.5 * (self.upper_left.x + self.upper_right.x),
            self.upper_left.y,
            mean_of_data(self.upper_left.value, self.upper_right.value),
        )
    }

    /// The segments of the contour at `level`, oriented with the higher
    /// values on their right. Both saddles get the same pair of segments, as
    /// in GDAL.
    fn segments(&self, level: f64) -> impl Iterator<Item = (Point, Point)> + '_ {
        use Border::*;
        let borders: &[(Border, Border)] = match self.marching_case(level) {
            ALL_LOW | ALL_HIGH => &[],
            UPPER_LEFT => &[(Upper, Left)],
            LOWER_LEFT => &[(Left, Lower)],
            LOWER_RIGHT => &[(Lower, Right)],
            UPPER_RIGHT => &[(Right, Upper)],
            UPPER_LEFT_LOWER_LEFT => &[(Upper, Lower)],
            LOWER_LEFT_LOWER_RIGHT => &[(Left, Right)],
            LOWER_RIGHT_UPPER_RIGHT => &[(Lower, Upper)],
            UPPER_RIGHT_UPPER_LEFT => &[(Right, Left)],
            ALL_BUT_UPPER_LEFT => &[(Left, Upper)],
            ALL_BUT_LOWER_LEFT => &[(Lower, Left)],
            ALL_BUT_LOWER_RIGHT => &[(Right, Lower)],
            ALL_BUT_UPPER_RIGHT => &[(Upper, Right)],
            // The two saddles.
            _ => &[(Left, Lower), (Right, Upper)],
        };
        borders.iter().map(move |&(start, end)| {
            (self.interpolate(start, level), self.interpolate(end, level))
        })
    }

    fn marching_case(&self, level: f64) -> u8 {
        let bit = |point: ValuedPoint, bit: u8| {
            if level < fudge(point.value, level) {
                bit
            } else {
                ALL_LOW
            }
        };
        bit(self.upper_left, UPPER_LEFT)
            | bit(self.lower_left, LOWER_LEFT)
            | bit(self.lower_right, LOWER_RIGHT)
            | bit(self.upper_right, UPPER_RIGHT)
    }

    /// The point of `border` at `level`.
    fn interpolate(&self, border: Border, level: f64) -> Point {
        let need_split = !self.split;
        let (ul, ll, lr, ur) = (
            self.upper_left,
            self.lower_left,
            self.lower_right,
            self.upper_right,
        );
        match border {
            Border::Left => Point {
                x: ul.x,
                y: interpolate(level, ll.y, ul.y, ll.value, ul.value, need_split),
            },
            Border::Lower => Point {
                x: interpolate(level, ll.x, lr.x, ll.value, lr.value, need_split),
                y: ll.y,
            },
            Border::Right => Point {
                x: ur.x,
                y: interpolate(level, lr.y, ur.y, lr.value, ur.value, need_split),
            },
            Border::Upper => Point {
                x: interpolate(level, ul.x, ur.x, ul.value, ur.value, need_split),
                y: ul.y,
            },
        }
    }
}

/// The mean of `a` and `b`, or the one that is not NaN.
fn mean_of_data(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        b
    } else if b.is_nan() {
        a
    } else {
        0.5 * (a + b)
    }
}

/// The coordinate between `x1` and `x2` where the value, linear from `y1` to
/// `y2`, is `level`.
///
/// For a whole square (`need_split`), it interpolates in the half of the side
/// that contains the level, from its midpoint, so that it matches the
/// sub-squares of a neighbouring square with nodata corners.
fn interpolate(
    level: f64,
    mut x1: f64,
    mut x2: f64,
    mut y1: f64,
    mut y2: f64,
    need_split: bool,
) -> f64 {
    if need_split {
        let xm = 0.5 * (x1 + x2);
        let ym = 0.5 * (y1 + y2);
        let fy1 = fudge(y1, level);
        let fym = fudge(ym, level);
        if (fy1 < level && level < fym) || (fy1 > level && level > fym) {
            x2 = xm;
            y2 = ym;
        } else {
            x1 = xm;
            y1 = ym;
        }
    }
    let fy1 = fudge(y1, level);
    let ratio = (level - fy1) / (fudge(y2, level) - fy1);
    x1 * (1.0 - ratio) + x2 * ratio
}

struct OpenLine {
    points: VecDeque<Point>,
    /// Whether a segment was added in the current row.
    merged: bool,
}

/// Joins segments into lines, per level.
#[derive(Default)]
struct SegmentMerger {
    /// The lines being built, per level index.
    open_lines: BTreeMap<usize, Vec<OpenLine>>,
    /// The finished lines with their level index, in emission order.
    lines: Vec<(usize, Vec<Point>)>,
}

impl SegmentMerger {
    fn beginning_of_line(&mut self) {
        for line in self.open_lines.values_mut().flatten() {
            line.merged = false;
        }
    }

    /// Emits the lines no segment of the row was added to: none will be.
    fn end_of_line(&mut self) {
        for (&level, lines) in &mut self.open_lines {
            let mut index = 0;
            while index < lines.len() {
                if lines[index].merged {
                    index += 1;
                } else {
                    let line = lines.remove(index);
                    self.lines.push((level, line.points.into()));
                }
            }
        }
    }

    fn finish(&mut self) {
        for (level, lines) in std::mem::take(&mut self.open_lines) {
            for line in lines {
                self.lines.push((level, line.points.into()));
            }
        }
    }

    fn add_segment(&mut self, level: usize, start: Point, end: Point) {
        let lines = self.open_lines.entry(level).or_default();
        if start == end {
            return;
        }

        // Extend the first line that ends at either point.
        let Some(index) = lines.iter_mut().position(|line| {
            let points = &mut line.points;
            if points.back() == Some(&end) {
                points.push_back(start);
            } else if points.front() == Some(&end) {
                points.push_front(start);
            } else if points.back() == Some(&start) {
                points.push_back(end);
            } else if points.front() == Some(&start) {
                points.push_front(end);
            } else {
                return false;
            }
            line.merged = true;
            true
        }) else {
            lines.push(OpenLine {
                points: VecDeque::from([start, end]),
                merged: true,
            });
            return;
        };

        // Join it with the first later line it now touches. A line that
        // closes is emitted.
        for other in index + 1..lines.len() {
            let (line, other_line) = (&lines[index].points, &lines[other].points);
            let joined = if line.back() == other_line.front() {
                let other_line = lines.remove(other);
                let line = &mut lines[index];
                line.points.pop_back();
                line.points.extend(other_line.points);
                index
            } else if other_line.back() == line.front() {
                let mut line = lines.remove(index);
                let other = other - 1;
                line.points.pop_front();
                let other_line = &mut lines[other];
                other_line.points.extend(line.points);
                other_line.merged = true;
                other
            } else if line.back() == other_line.back() {
                let other_line = lines.remove(other);
                let line = &mut lines[index];
                line.points.pop_back();
                line.points.extend(other_line.points.into_iter().rev());
                index
            } else if line.front() == other_line.front() {
                let other_line = lines.remove(other);
                let line = &mut lines[index];
                line.points.pop_front();
                for point in other_line.points {
                    line.points.push_front(point);
                }
                index
            } else {
                continue;
            };
            let points = &lines[joined].points;
            if points.front() == points.back() {
                let line = lines.remove(joined);
                self.lines.push((level, line.points.into()));
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODATA: f32 = -9999.0;

    fn grid(width: usize, height: usize, values: Vec<f32>) -> Grid<f32> {
        Grid {
            min_x: 1000.0,
            max_y: 2000.0,
            resolution: 2.0,
            width,
            height,
            nodata: Some(NODATA),
            values,
        }
    }

    #[test]
    fn computes_levels_between_min_and_max() {
        assert_eq!(
            levels(&[1.0, 7.5, f64::NAN], 2.5),
            Some(vec![2.5, 5.0, 7.5])
        );
        // As in GDAL, a minimum or maximum on a level is moved up by the
        // fudge tolerance, so the level is out at the minimum and in at the
        // maximum.
        assert_eq!(levels(&[2.5, 6.0], 2.5), Some(vec![5.0]));
        assert_eq!(levels(&[1.0, 5.0], 2.5), Some(vec![2.5, 5.0]));
        assert_eq!(levels(&[3.0, 4.0], 2.5), None);
        assert_eq!(levels(&[3.0, 3.0], 2.5), None);
        assert_eq!(levels(&[f64::NAN], 2.5), None);
    }

    /// A 9 x 7 DEM with a peak, a pit, a saddle, values on a level and a
    /// nodata hole that cuts some contours.
    fn sample_dem() -> Grid<f32> {
        let mut dem = grid(9, 7, Vec::new());
        dem.values = (0..63)
            .map(|i| {
                let (col, row) = ((i % 9) as f32, (i / 9) as f32);
                let peak = 9.0 - ((col - 2.0).powi(2) + (row - 2.0).powi(2)) * 0.8;
                let pit = ((col - 6.0).powi(2) + (row - 4.0).powi(2)) * 0.7;
                peak.max(0.0) + pit.min(6.0) + if i % 11 == 0 { 2.5 } else { 0.0 }
            })
            .collect();
        dem.values[2 * 9 + 6] = 5.0;
        for (col, row) in [(4, 4), (4, 5), (5, 5), (8, 0)] {
            dem.set(col, row, NODATA);
        }
        dem
    }

    #[test]
    fn matches_gdal_contour() {
        // From `gdal_contour -a elev -i 2.5` (GDAL 3.12) on `sample_dem`
        // written as a Float32 GeoTIFF, printed by `ogrinfo` with 15
        // significant digits: elevation, then x,y pairs.
        let expected = "
        15 1005.00000249999 1995.0,1005.0 1994.99999750001,1004.99999750001 1995.0,1003.96969686461 1997.0,1005.0 1997.6938777814,1006.03030313539 1997.0,1005.00000249999 1995.0
        5 1018.0 1994.42857149345,1017.0 1994.42857149345,1016.42857149345 1995.0,1015.0 1996.2,1013.00000133333 1995.0,1013.0 1994.99999888889,1011.0 1994.79310361837,1010.14754100155 1993.0,1010.0 1992.71578944713,1009.75539568799 1992.0
        7.5 1009.84615401545 2000.0,1009.84615401545 1999.0,1010.75 1997.0,1010.48837202081 1995.0,1009.32786885758 1993.0,1009.21582733943 1992.0
        10 1007.83333346579 2000.0,1007.83333346579 1999.0,1009.0 1997.83333346579,1009.5 1997.0,1009.90697666553 1995.0,1009.0 1993.5555555948,1008.38775505438 1993.0,1008.39189186054 1992.0
        12.5 1007.36734681167 1993.0,1007.0 1992.25000019868,1005.0 1991.58333326711,1003.0 1992.25000019868,1002.25000019868 1993.0,1001.58333326711 1995.0,1002.25000019868 1997.0,1003.0 1997.74999980132,1005.0 1998.7142858255,1007.0 1997.74999980132,1007.74999980132 1997.0,1009.0 1995.9655171053,1009.32558131025 1995.0,1009.0 1994.48148158614,1007.36734681167 1993.0
        5 1018.0 1990.66666649006,1017.0 1990.66666649006,1016.86956514021 1991.0,1017.0 1991.33333350994,1018.0 1991.33333350994
        10 1008.0 1990.5,1007.0 1990.5,1005.0 1990.10000001192,1003.0 1990.5,1002.16666653421 1991.0,1001.0 1992.16666653421,1000.0 1992.16666653421
        2.5 1011.85714284255 1988.0,1012.0 1987.79591834203,1013.0 1987.28571424517,1015.0 1987.95238094157,1016.04761905843 1989.0,1015.78260866636 1991.0,1016.04761905843 1993.0,1015.0 1994.04761905843,1013.77777779985 1993.0,1013.0 1992.56249997672,1011.24999977648 1993.0,1011.0 1993.06896546054,1010.96721314551 1993.0,1010.5899280731 1992.0,1010.0 1991.56842104926
        7.5 1008.0 1989.25,1007.0 1989.25,1005.74999955297 1989.0,1005.0 1988.66666693158,1004.25000044703 1989.0,1003.0 1989.25,1001.0 1990.15384598455,1000.0 1990.15384598455
        5 1009.57142850655 1988.0,1009.57142850655 1987.0,1009.57142850655 1986.0
        5 1016.42857149345 1986.0,1016.42857149345 1987.0,1017.0 1987.57142850655,1018.0 1987.57142850655
        7.5 1002.2 1986.0,1002.2 1987.0,1003.0 1988.33333333333,1003.8 1987.0,1003.8 1986.0
        ";
        let lines = contour_lines(&sample_dem(), 2.5);
        let expected: Vec<&str> = expected.trim().lines().map(str::trim).collect();
        assert_eq!(lines.len(), expected.len());
        for (line, expected) in lines.iter().zip(expected) {
            let (elevation, points) = expected.split_once(' ').unwrap();
            assert_eq!(line.elevation, elevation.parse::<f64>().unwrap());
            let points: Vec<(f64, f64)> = points
                .split(',')
                .map(|point| {
                    let (x, y) = point.split_once(' ').unwrap();
                    (x.parse().unwrap(), y.parse().unwrap())
                })
                .collect();
            assert_eq!(line.points.len(), points.len(), "{expected}");
            for (&(x, y), &(expected_x, expected_y)) in line.points.iter().zip(&points) {
                let close = (x - expected_x).abs() < 1e-9 && (y - expected_y).abs() < 1e-9;
                assert!(close, "{expected}");
            }
        }
    }
}
