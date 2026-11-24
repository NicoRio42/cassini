use crate::error::{CassiniError, Result, ResultContext};
use std::{fs::File, path::Path};
use tiff::{
    decoder::{Decoder, DecodingResult},
    tags::Tag,
};

/// A 32-bit floating-point TIFF band, row-major from the north-west corner.
pub struct F32Raster {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
    /// The value of the cells without data, from the GDAL_NODATA tag.
    pub no_data: Option<f32>,
}

impl F32Raster {
    /// Reads the first band of the TIFF file at `path`.
    pub fn read(path: &Path) -> Result<Self> {
        let file = File::open(path).context(format!("could not open `{}`", path.display()))?;
        let mut decoder = Decoder::new(file)
            .context(format!("could not create decoder for `{}`", path.display()))?
            .with_limits(tiff::decoder::Limits::unlimited());

        let (width, height) = decoder.dimensions().context(format!(
            "could not read dimensions from `{}`",
            path.display()
        ))?;
        let no_data = decoder
            .get_tag_ascii_string(Tag::GdalNodata)
            .ok()
            .and_then(|value| value.trim().parse().ok());

        let DecodingResult::F32(values) = decoder
            .read_image()
            .context(format!("could not decode `{}`", path.display()))?
        else {
            return Err(CassiniError::InvalidArtifact {
                path: path.to_path_buf(),
                message: "expected a 32-bit floating-point TIFF band".to_owned(),
            });
        };
        let (width, height) = (width as usize, height as usize);
        if values.len() != width * height {
            return Err(CassiniError::InvalidArtifact {
                path: path.to_path_buf(),
                message: format!(
                    "expected {} values for {width} x {height} cells, found {}",
                    width * height,
                    values.len()
                ),
            });
        }

        Ok(Self {
            width,
            height,
            values,
            no_data,
        })
    }

    /// Whether `value` is a cell with data.
    pub fn has_data(&self, value: f32) -> bool {
        !value.is_nan() && self.no_data != Some(value)
    }
}
