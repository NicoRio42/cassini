use std::{error::Error as StdError, fmt, path::PathBuf, process::ExitStatus};

pub type Result<T> = std::result::Result<T, CassiniError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Lidar,
    Buffer,
    Dem,
    Contours,
    Vectors,
    Download,
    Render,
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Lidar => "LiDAR processing",
            Self::Buffer => "raster buffering",
            Self::Dem => "DEM processing",
            Self::Contours => "contour generation",
            Self::Vectors => "vector rendering",
            Self::Download => "vector download",
            Self::Render => "map rendering",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileId {
    pub min_x: i64,
    pub min_y: i64,
    pub max_x: i64,
    pub max_y: i64,
}

impl fmt::Display for TileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "({}, {})-({}, {})",
            self.min_x, self.min_y, self.max_x, self.max_y
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CassiniError {
    #[error("invalid input: {message}")]
    InvalidInput { message: String },

    #[error("invalid configuration: {message}")]
    InvalidConfig { message: String },

    #[error("invalid artifact `{path}`: {message}")]
    InvalidArtifact { path: PathBuf, message: String },

    #[error("request to `{url}` returned HTTP status {status}")]
    HttpStatus { url: String, status: u16 },

    #[error("{context}")]
    Operation {
        context: String,
        #[source]
        source: Box<dyn StdError + Send + Sync>,
    },

    #[error("could not start `{program}` during {stage}{tile_suffix}")]
    CommandStart {
        program: String,
        stage: Stage,
        tile_suffix: String,
        #[source]
        source: std::io::Error,
    },

    #[error("`{program}` failed during {stage}{tile_suffix} with {status}: {stderr}")]
    CommandFailed {
        program: String,
        stage: Stage,
        tile_suffix: String,
        status: ExitStatus,
        stderr: String,
    },

    #[error("worker panicked during {stage}{tile_suffix}: {message}")]
    WorkerPanicked {
        stage: Stage,
        tile_suffix: String,
        message: String,
    },

    #[error("batch processing failed for {failure_count} tile(s)")]
    Batch {
        failure_count: usize,
        failures: Vec<CassiniError>,
    },
}

impl CassiniError {
    pub fn operation(
        context: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::Operation {
            context: context.into(),
            source: Box::new(source),
        }
    }

    pub fn tile_suffix(tile: Option<TileId>) -> String {
        tile.map(|tile| format!(" for tile {tile}"))
            .unwrap_or_default()
    }
}

pub trait ResultContext<T> {
    fn context(self, context: impl Into<String>) -> Result<T>;
}

impl<T, E> ResultContext<T> for std::result::Result<T, E>
where
    E: StdError + Send + Sync + 'static,
{
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.map_err(|source| CassiniError::operation(context, source))
    }
}
