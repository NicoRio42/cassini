use std::{
    path::Path,
    process::{Command, Output},
};

use crate::error::{CassiniError, Result, Stage, TileId};

pub fn checked_output(command: &mut Command, stage: Stage, tile: Option<TileId>) -> Result<Output> {
    let program = command.get_program().to_string_lossy().into_owned();
    let tile_suffix = CassiniError::tile_suffix(tile);
    let output = command
        .output()
        .map_err(|source| CassiniError::CommandStart {
            program: program.clone(),
            stage,
            tile_suffix: tile_suffix.clone(),
            source,
        })?;

    if !output.status.success() {
        return Err(CassiniError::CommandFailed {
            program,
            stage,
            tile_suffix,
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }

    Ok(output)
}

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

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::checked_output;
    use crate::error::{CassiniError, Stage};

    #[test]
    fn non_zero_exit_is_a_command_error() {
        let error = checked_output(
            Command::new("sh").args(["-c", "printf boom >&2; exit 7"]),
            Stage::Vectors,
            None,
        )
        .unwrap_err();

        match error {
            CassiniError::CommandFailed { status, stderr, .. } => {
                assert_eq!(status.code(), Some(7));
                assert_eq!(stderr, "boom");
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }
    }
}
