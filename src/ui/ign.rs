//! IGN's LiDAR HD tile index: the 1 km tiles of an area, and the URLs of
//! their LAZ file and DEM.

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::{
    download::USER_AGENT_VALUE,
    error::{CassiniError, Result, ResultContext},
    projection::{lambert93_to_wgs84, wgs84_to_lambert93},
};
use reqwest::{header::USER_AGENT, Url};

const WFS_URL: &str = "https://data.geopf.fr/wfs/wfs";
const WFS_LAYER: &str = "IGNF_LIDAR-HD_METADONNEE:metadata";
const WFS_PAGE_SIZE: usize = 1000;
const TILE_SIDE: f64 = 1000.0;
/// The largest bounding box a selection may have, in km². IGN's tiles are
/// about 250 MB each, so this is far more than anyone downloads.
const MAX_SELECTION_AREA_KM2: f64 = 2500.0;

/// A LiDAR HD tile of IGN's index.
#[derive(Clone, Debug, Serialize)]
pub struct IgnTile {
    /// The kilometre coordinates of its north-west corner, `0914_6439`.
    pub id: String,
    pub min_x: i64,
    pub max_y: i64,
    pub laz_url: String,
    pub dem_url: String,
    pub laz_file_name: String,
    pub dem_file_name: String,
    pub points: u64,
    /// Its corners in longitudes and latitudes: north-west, north-east,
    /// south-east, south-west.
    pub corners: [[f64; 2]; 4],
}

/// A GeoJSON geometry in longitudes and latitudes, as drawn on the map.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum Geometry {
    Point {
        coordinates: [f64; 2],
    },
    Polygon {
        coordinates: Vec<Vec<[f64; 2]>>,
    },
    MultiPolygon {
        coordinates: Vec<Vec<Vec<[f64; 2]>>>,
    },
}

/// A selection in Lambert-93.
enum Selection {
    Point(f64, f64),
    /// The outer rings of the polygons: the selection has no holes.
    Rings(Vec<Vec<(f64, f64)>>),
}

impl Selection {
    fn from_geometry(geometry: &Geometry) -> Result<Self> {
        let to_lambert93 = |ring: &Vec<[f64; 2]>| -> Vec<(f64, f64)> {
            ring.iter()
                .map(|&[lon, lat]| wgs84_to_lambert93(lon, lat))
                .collect()
        };
        let selection = match geometry {
            Geometry::Point {
                coordinates: [lon, lat],
            } => {
                let (x, y) = wgs84_to_lambert93(*lon, *lat);
                Self::Point(x, y)
            }
            Geometry::Polygon { coordinates } => {
                Self::Rings(coordinates.first().map(to_lambert93).into_iter().collect())
            }
            Geometry::MultiPolygon { coordinates } => Self::Rings(
                coordinates
                    .iter()
                    .filter_map(|polygon| polygon.first().map(to_lambert93))
                    .collect(),
            ),
        };

        let valid = match &selection {
            Selection::Point(x, y) => x.is_finite() && y.is_finite(),
            Selection::Rings(rings) => {
                !rings.is_empty()
                    && rings.iter().all(|ring| {
                        ring.len() >= 3 && ring.iter().all(|(x, y)| x.is_finite() && y.is_finite())
                    })
            }
        };
        if !valid {
            return Err(CassiniError::InvalidInput {
                message: "the selection is not a valid point or polygon".to_owned(),
            });
        }
        Ok(selection)
    }

    /// Its bounding box, `min_x, min_y, max_x, max_y`.
    fn bounds(&self) -> (f64, f64, f64, f64) {
        match self {
            Selection::Point(x, y) => (*x, *y, *x, *y),
            Selection::Rings(rings) => rings.iter().flatten().fold(
                (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
                |(min_x, min_y, max_x, max_y), &(x, y)| {
                    (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                },
            ),
        }
    }

    fn intersects_tile(&self, min_x: f64, max_y: f64) -> bool {
        match self {
            // Points on an edge go east or north, as in the LiDAR binning.
            Selection::Point(x, y) => {
                min_x <= *x && *x < min_x + TILE_SIDE && max_y - TILE_SIDE <= *y && *y < max_y
            }
            Selection::Rings(rings) => rings
                .iter()
                .any(|ring| ring_intersects_square(ring, min_x, max_y - TILE_SIDE, TILE_SIDE)),
        }
    }
}

/// The tiles of IGN's index that the geometry touches, sorted by id.
pub fn tiles_in_geometry(client: &Client, geometry: &Geometry) -> Result<Vec<IgnTile>> {
    let selection = Selection::from_geometry(geometry)?;
    let (min_x, min_y, max_x, max_y) = selection.bounds();
    let area_km2 = (max_x - min_x) * (max_y - min_y) / 1_000_000.0;
    if area_km2 > MAX_SELECTION_AREA_KM2 {
        return Err(CassiniError::InvalidInput {
            message: format!(
                "the selection spans {area_km2:.0} km², more than the {MAX_SELECTION_AREA_KM2:.0} km² allowed"
            ),
        });
    }

    let mut tiles: Vec<IgnTile> = fetch_tiles_in_bbox(client, min_x, min_y, max_x, max_y)?
        .into_iter()
        .filter(|tile| selection.intersects_tile(tile.min_x as f64, tile.max_y as f64))
        .collect();
    tiles.sort_by(|a, b| a.id.cmp(&b.id));
    tiles.dedup_by(|a, b| a.id == b.id);
    Ok(tiles)
}

#[derive(Deserialize)]
struct FeatureCollection {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Properties,
}

#[derive(Deserialize)]
struct Properties {
    coordonnees_nw: String,
    url_npl: Option<String>,
    url_mnt: Option<String>,
    nombre_points: Option<u64>,
}

/// The tiles whose footprint intersects the Lambert-93 bounding box, from
/// IGN's WFS, a page at a time.
fn fetch_tiles_in_bbox(
    client: &Client,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
) -> Result<Vec<IgnTile>> {
    let mut tiles = vec![];
    for page in 0.. {
        let start_index = (page * WFS_PAGE_SIZE).to_string();
        let count = WFS_PAGE_SIZE.to_string();
        let bbox = format!("{min_x},{min_y},{max_x},{max_y},urn:ogc:def:crs:EPSG::2154");
        let url = Url::parse_with_params(
            WFS_URL,
            &[
                ("SERVICE", "WFS"),
                ("VERSION", "2.0.0"),
                ("REQUEST", "GetFeature"),
                ("TYPENAMES", WFS_LAYER),
                ("OUTPUTFORMAT", "application/json"),
                ("SRSNAME", "EPSG:2154"),
                ("BBOX", &bbox),
                ("COUNT", &count),
                ("STARTINDEX", &start_index),
            ],
        )
        .context("could not build the tile index URL")?;
        let response = client
            .get(url)
            .header(USER_AGENT, USER_AGENT_VALUE)
            .send()
            .context("could not query IGN's LiDAR HD tile index")?;
        let status = response.status();
        if !status.is_success() {
            return Err(CassiniError::HttpStatus {
                url: WFS_URL.to_owned(),
                status: status.as_u16(),
            });
        }
        let body = response
            .bytes()
            .context("could not read IGN's LiDAR HD tile index")?;
        let collection: FeatureCollection =
            serde_json::from_slice(&body).context("could not parse IGN's LiDAR HD tile index")?;
        let page_len = collection.features.len();
        tiles.extend(
            collection
                .features
                .into_iter()
                .filter_map(|feature| tile_from_properties(feature.properties)),
        );
        if page_len < WFS_PAGE_SIZE {
            break;
        }
    }
    Ok(tiles)
}

/// The tile of a WFS feature, or `None` if it has no LAZ file or DEM, or
/// fields Cassini doesn't understand.
fn tile_from_properties(properties: Properties) -> Option<IgnTile> {
    let (x_km, y_km) = properties.coordonnees_nw.split_once('-')?;
    let (x_km, y_km): (u32, u32) = (x_km.parse().ok()?, y_km.parse().ok()?);
    let laz_url = https(properties.url_npl?);
    let dem_url = https(properties.url_mnt?);
    let laz_file_name = laz_url.rsplit('/').next()?.to_owned();
    if !is_safe_file_name(&laz_file_name) || !laz_file_name.to_lowercase().ends_with(".laz") {
        return None;
    }

    let min_x = i64::from(x_km) * 1000;
    let max_y = i64::from(y_km) * 1000;
    let corner = |x: i64, y: i64| {
        let (lon, lat) = lambert93_to_wgs84(x as f64, y as f64);
        [lon, lat]
    };
    let (max_x, min_y) = (min_x + TILE_SIDE as i64, max_y - TILE_SIDE as i64);
    Some(IgnTile {
        id: format!("{x_km:04}_{y_km:04}"),
        min_x,
        max_y,
        laz_url,
        dem_url,
        laz_file_name,
        dem_file_name: format!("LHD_FXX_{x_km:04}_{y_km:04}_MNT_O_0M50_LAMB93_IGN69.tif"),
        points: properties.nombre_points.unwrap_or(0),
        corners: [
            corner(min_x, max_y),
            corner(max_x, max_y),
            corner(max_x, min_y),
            corner(min_x, min_y),
        ],
    })
}

/// Some of the index's URLs are `http://`, although the server redirects.
fn https(url: String) -> String {
    match url.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => url,
    }
}

/// Whether a name taken from the index can be used as a file name in the
/// input directory.
pub fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Whether the polygon `ring` overlaps the inside of the square of side
/// `side` whose south-west corner is (`min_x`, `min_y`). Touching its edge
/// isn't overlapping, so a rectangle drawn along tile edges doesn't take the
/// tiles around it.
fn ring_intersects_square(ring: &[(f64, f64)], min_x: f64, min_y: f64, side: f64) -> bool {
    let (max_x, max_y) = (min_x + side, min_y + side);
    let strictly_inside = |(x, y): (f64, f64)| min_x < x && x < max_x && min_y < y && y < max_y;

    // The square inside the polygon, or an edge of the polygon crossing it.
    point_in_ring((min_x + side / 2.0, min_y + side / 2.0), ring)
        || ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .filter_map(|(&a, &b)| clip_segment(a, b, min_x, min_y, max_x, max_y))
            .any(|(a, b)| strictly_inside(((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)))
}

/// The part of the segment from `a` to `b` inside the rectangle, by
/// Liang-Barsky, or `None` if it misses it.
fn clip_segment(
    a: (f64, f64),
    b: (f64, f64),
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
) -> Option<((f64, f64), (f64, f64))> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let mut t0: f64 = 0.0;
    let mut t1: f64 = 1.0;
    for (p, q) in [
        (-dx, a.0 - min_x),
        (dx, max_x - a.0),
        (-dy, a.1 - min_y),
        (dy, max_y - a.1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    let point = |t: f64| (a.0 + t * dx, a.1 + t * dy);
    (t0 <= t1).then(|| (point(t0), point(t1)))
}

/// Whether the point is inside the ring, by ray casting.
fn point_in_ring((x, y): (f64, f64), ring: &[(f64, f64)]) -> bool {
    let mut inside = false;
    for (&(xi, yi), &(xj, yj)) in ring.iter().zip(ring.iter().cycle().skip(1)) {
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(min_x: f64, min_y: f64, side: f64) -> Vec<(f64, f64)> {
        vec![
            (min_x, min_y),
            (min_x + side, min_y),
            (min_x + side, min_y + side),
            (min_x, min_y + side),
        ]
    }

    #[test]
    fn a_rectangle_along_tile_edges_takes_only_the_tiles_inside() {
        let rectangle = square(1000.0, 1000.0, 2000.0);
        let tiles: Vec<(i32, i32)> = (-1..4)
            .flat_map(|i| (-1..4).map(move |j| (i, j)))
            .filter(|&(i, j)| {
                ring_intersects_square(&rectangle, i as f64 * 1000.0, j as f64 * 1000.0, 1000.0)
            })
            .collect();
        assert_eq!(tiles, vec![(1, 1), (1, 2), (2, 1), (2, 2)]);
    }

    #[test]
    fn a_polygon_inside_a_tile_or_crossing_it_intersects_it() {
        assert!(ring_intersects_square(
            &square(100.0, 100.0, 10.0),
            0.0,
            0.0,
            1000.0
        ));
        // A thin triangle crossing the tile with no vertex inside it.
        let triangle = vec![(-500.0, 400.0), (1500.0, 500.0), (-500.0, 600.0)];
        assert!(ring_intersects_square(&triangle, 0.0, 0.0, 1000.0));
        assert!(!ring_intersects_square(&triangle, 0.0, 1000.0, 1000.0));
    }

    #[test]
    fn a_point_on_an_edge_goes_to_the_tile_east_and_north_of_it() {
        let point = Selection::Point(1000.0, 2000.0);
        assert!(point.intersects_tile(1000.0, 3000.0));
        assert!(!point.intersects_tile(0.0, 3000.0));
        assert!(!point.intersects_tile(1000.0, 2000.0));
    }

    #[test]
    fn reads_the_tile_of_a_wfs_feature() {
        let tile = tile_from_properties(Properties {
            coordonnees_nw: "0914-6439".to_owned(),
            url_npl: Some(
                "http://data.geopf.fr/telechargement/download/LiDARHD-NUALID/x/LHD_FXX_0914_6439_PTS_LAMB93_IGN69.copc.laz"
                    .to_owned(),
            ),
            url_mnt: Some("https://data.geopf.fr/wms-r?BBOX=1".to_owned()),
            nombre_points: Some(42),
        })
        .unwrap();
        assert_eq!(tile.id, "0914_6439");
        assert_eq!((tile.min_x, tile.max_y), (914_000, 6_439_000));
        assert!(tile.laz_url.starts_with("https://data.geopf.fr/"));
        assert_eq!(
            tile.laz_file_name,
            "LHD_FXX_0914_6439_PTS_LAMB93_IGN69.copc.laz"
        );
        assert_eq!(
            tile.dem_file_name,
            "LHD_FXX_0914_6439_MNT_O_0M50_LAMB93_IGN69.tif"
        );
    }

    #[test]
    fn rejects_file_names_that_leave_the_input_directory() {
        assert!(!is_safe_file_name("../x.laz"));
        assert!(!is_safe_file_name("a/b.laz"));
        assert!(!is_safe_file_name(".hidden.laz"));
        assert!(is_safe_file_name(
            "LHD_FXX_0914_6439_PTS_LAMB93_IGN69.copc.laz"
        ));
    }
}
