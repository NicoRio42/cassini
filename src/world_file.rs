use std::path::Path;

/// The georeferencing of an image without rotation, in a world file: an
/// image file name with a `w` replacing the last letter of its extension,
/// such as `full-map.pgw`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldFile {
    /// The ground size of a pixel, in metres.
    pub pixel_size: f64,
    /// The world position of the centre of the north-west pixel, as GDAL,
    /// QGIS and OCAD read it.
    pub center_x: f64,
    pub center_y: f64,
}

impl WorldFile {
    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_string())
    }
}

impl std::fmt::Display for WorldFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:.10}\n0.0\n0.0\n{:.10}\n{:.6}\n{:.6}",
            self.pixel_size, -self.pixel_size, self.center_x, self.center_y
        )
    }
}

#[cfg(test)]
mod tests {
    use super::WorldFile;

    #[test]
    fn writes_the_pixel_size_and_the_centre_of_the_north_west_pixel() {
        let world_file = WorldFile {
            pixel_size: 254.0 / 600.0,
            center_x: 899_999.903_666_7,
            center_y: 6_401_000.137_666_7,
        };
        assert_eq!(
            world_file.to_string(),
            "0.4233333333\n0.0\n0.0\n-0.4233333333\n899999.903667\n6401000.137667"
        );
    }
}
