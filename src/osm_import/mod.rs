//! The `lines` and `multipolygons` layers of a tile's OpenStreetMap file,
//! as `ogr2ogr -t_srs EPSG:2154 -sql "SELECT * FROM <layer>"` read them
//! with OGR's OSM driver and its default `osmconf.ini`: the same features in
//! the same order, with the same tags. The vector step reads them with
//! [`read_osm_file`]; [`convert_osm_file_to_shapefiles`] writes them as the
//! shapefiles `ogr2ogr` wrote.
//!
//! Ported from GDAL 3.12: the OSM driver (`ogr/ogrsf_frmts/osm`), the ring
//! assembly of `OGRBuildPolygonFromEdges` (`ogr/ograssemblepolygon.cpp`) and
//! `OGRGeometryFactory::organizePolygons` (`ogr/ogrgeometryfactory.cpp`):
//!
//! > Copyright (c) 1999, Frank Warmerdam
//! > Copyright (c) 2008-2014, Even Rouault <even dot rouault at spatialys.com>
//! >
//! > SPDX-License-Identifier: MIT
//!
//! OGR resolves the nodes of the ways it has read so far when it reaches the
//! first relation or the end of the file, so it only knows every node if the
//! relations come after them. Here the whole file is read first, and ways
//! are resolved against all its nodes, wherever the relations are.
//!
//! The `.dbf` is UTF-8, with a `.cpg` saying so, where OGR recodes to
//! ISO-8859-1. Values longer than a field are cut at 254 bytes, as the dBase
//! format requires, on a character boundary.

mod layers;
mod parse;
mod rings;
mod shapefiles;

use std::{fs, path::Path};

use log::warn;
use shapefile::{Point, Polygon, PolygonRing, Polyline};

use crate::{
    error::{CassiniError, Result, ResultContext},
    projection::wgs84_to_lambert93,
};

/// An OSM tag: key and value.
pub type Tag = (String, String);

/// The features of an OSM file.
#[derive(Debug, Default)]
pub struct OsmLayers {
    pub lines: Vec<LineFeature>,
    pub multipolygons: Vec<AreaFeature>,
}

/// A way of the `lines` layer.
#[derive(Debug)]
pub struct LineFeature {
    pub way_id: i64,
    pub tags: Vec<Tag>,
    points: Vec<LonLat>,
}

impl LineFeature {
    /// The line in Lambert-93.
    pub fn to_polyline(&self) -> Polyline {
        Polyline::new(
            self.points
                .iter()
                .map(|point| point.to_lambert93())
                .collect(),
        )
    }
}

/// A feature of the `multipolygons` layer.
#[derive(Debug)]
pub struct AreaFeature {
    pub source: AreaSource,
    pub tags: Vec<Tag>,
    polygons: Vec<PolygonRings>,
}

impl AreaFeature {
    /// The polygons in Lambert-93, with their outer rings clockwise.
    pub fn to_polygon(&self) -> Polygon {
        let to_lambert93 = |ring: &Ring| ring.iter().map(|point| point.to_lambert93()).collect();
        let rings = self
            .polygons
            .iter()
            .flat_map(|polygon| {
                [PolygonRing::Outer(to_lambert93(&polygon.exterior))]
                    .into_iter()
                    .chain(
                        polygon
                            .interiors
                            .iter()
                            .map(|ring| PolygonRing::Inner(to_lambert93(ring))),
                    )
            })
            .collect();
        Polygon::with_rings(rings)
    }
}

/// The OSM element an area comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AreaSource {
    Relation(i64),
    Way(i64),
}

/// A position in 1e-7 degrees, as OGR stores node coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LonLat {
    lon: i32,
    lat: i32,
}

impl LonLat {
    fn from_degrees(lon: f64, lat: f64) -> Self {
        let fixed = |degrees: f64| (degrees * 1.0e7 + 0.5).floor() as i32;
        Self {
            lon: fixed(lon),
            lat: fixed(lat),
        }
    }

    fn x(self) -> f64 {
        f64::from(self.lon) / 1.0e7
    }

    fn y(self) -> f64 {
        f64::from(self.lat) / 1.0e7
    }

    fn to_lambert93(self) -> Point {
        let (x, y) = wgs84_to_lambert93(self.x(), self.y());
        Point::new(x, y)
    }
}

type Ring = Vec<LonLat>;

/// A polygon and its holes.
#[derive(Debug, Clone, PartialEq)]
struct PolygonRings {
    exterior: Ring,
    interiors: Vec<Ring>,
}

/// Reads the `lines` and `multipolygons` layers of the OSM XML file
/// `osm_path`.
pub fn read_osm_file(osm_path: &Path) -> Result<OsmLayers> {
    let xml = fs::read_to_string(osm_path)
        .context(format!("could not read OSM file `{}`", osm_path.display()))?;
    let data = parse::parse_osm(&xml).map_err(|message| CassiniError::InvalidArtifact {
        path: osm_path.to_path_buf(),
        message: format!("{message} (delete it to download it again)"),
    })?;
    for remark in &data.remarks {
        warn!(
            "OSM file `{}` may be incomplete, delete it to download it again. Overpass remark: {remark}",
            osm_path.display()
        );
    }
    Ok(layers::build_layers(&data))
}

/// Writes the `lines.shp` and `multipolygons.shp` layers of the OSM XML
/// file `osm_path` to `output_dir`, in Lambert-93.
pub fn convert_osm_file_to_shapefiles(osm_path: &Path, output_dir: &Path) -> Result<()> {
    let layers = read_osm_file(osm_path)?;
    fs::create_dir_all(output_dir).context(format!(
        "could not create shapefile directory `{}`",
        output_dir.display()
    ))?;
    shapefiles::write_lines(&output_dir.join("lines.shp"), &layers.lines)?;
    shapefiles::write_multipolygons(&output_dir.join("multipolygons.shp"), &layers.multipolygons)
}

#[cfg(test)]
mod test_support {
    use super::{LonLat, Ring, Tag};

    pub fn tags(pairs: &[(&str, &str)]) -> Vec<Tag> {
        pairs
            .iter()
            .map(|&(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }

    pub fn ring(corners: &[(f64, f64)]) -> Ring {
        corners
            .iter()
            .map(|&(lon, lat)| LonLat::from_degrees(lon, lat))
            .collect()
    }
}
