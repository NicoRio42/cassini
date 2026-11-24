use std::path::Path;

use crate::error::{CassiniError, Result};

pub fn ensure_file(path: &Path) -> Result<()> {
    if path.is_file() {
        Ok(())
    } else {
        Err(CassiniError::InvalidArtifact {
            path: path.to_path_buf(),
            message: "expected output file was not created".to_owned(),
        })
    }
}
