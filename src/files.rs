//! The files Cassini reads from and writes to directories the user chooses.

use std::{
    fs::{remove_dir, remove_dir_all, remove_file},
    io::ErrorKind,
    path::Path,
};

use crate::error::{CassiniError, Result, ResultContext};

/// The extensions of the LiDAR files that `batch` reads.
pub const LAZ_EXTENSIONS: [&str; 1] = ["laz"];

/// The extensions of the DEM files that `batch` reads.
pub const GEOTIFF_EXTENSIONS: [&str; 2] = ["tif", "tiff"];

/// The extensions of the files of a shapefile.
const SHAPEFILE_EXTENSIONS: [&str; 5] = ["shp", "shx", "dbf", "prj", "cpg"];

/// Whether `path` is a file with one of `extensions`, in any case, that is
/// not hidden. macOS writes hidden `._` files next to the files it copies to
/// some drives, with the same extension.
pub fn is_input_file(path: &Path, extensions: &[&str]) -> bool {
    let is_hidden = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'));
    let has_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extensions
                .iter()
                .any(|expected| extension.eq_ignore_ascii_case(expected))
        });
    has_extension && !is_hidden && path.is_file()
}

/// Removes the files at the relative paths `names` in `dir_path`, if they
/// exist, leaving any other entry of `dir_path` in place. Fails on a
/// directory: `dir_path` can be any directory the user chooses, so it never
/// removes one recursively.
pub fn remove_artifacts(dir_path: &Path, names: &[&str]) -> Result<()> {
    for name in names {
        let artifact = dir_path.join(name);
        match remove_file(&artifact) {
            Err(error) if error.kind() != ErrorKind::NotFound => {
                return Err(error).context(format!(
                    "could not remove stale artifact `{}`",
                    artifact.display()
                ))
            }
            _ => {}
        }
    }
    Ok(())
}

/// Removes the shapefiles at the relative paths `paths` in `dir_path`,
/// without extension, then their directories if nothing else is left in
/// them.
pub fn remove_shapefile_artifacts(dir_path: &Path, paths: &[&str]) -> Result<()> {
    for path in paths {
        let files: Vec<String> = SHAPEFILE_EXTENSIONS
            .iter()
            .map(|extension| format!("{path}.{extension}"))
            .collect();
        let files: Vec<&str> = files.iter().map(String::as_str).collect();
        remove_artifacts(dir_path, &files)?;

        if let Some(parent) = Path::new(path)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
        {
            let parent = dir_path.join(parent);
            match remove_dir(&parent) {
                Err(error)
                    if error.kind() != ErrorKind::NotFound
                        && error.kind() != ErrorKind::DirectoryNotEmpty =>
                {
                    return Err(error).context(format!(
                        "could not remove stale artifact directory `{}`",
                        parent.display()
                    ))
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Fails if `input`, a file the user gave, is one of the artifacts `names`
/// that a stage is about to replace in `dir_path`.
pub fn check_not_an_artifact(input: &Path, dir_path: &Path, names: &[&str]) -> Result<()> {
    let Ok(input) = input.canonicalize() else {
        return Ok(());
    };
    for name in names {
        let artifact = dir_path.join(name);
        if artifact
            .canonicalize()
            .is_ok_and(|artifact| artifact == input)
        {
            return Err(CassiniError::InvalidInput {
                message: format!(
                    "`{}` would be replaced by the output `{name}`: move it, or choose another \
                     output directory",
                    input.display()
                ),
            });
        }
    }
    Ok(())
}

/// Removes the file or directory at `path`, if it exists.
pub fn remove_if_exists<P: AsRef<Path>>(path: P) -> std::io::Result<()> {
    let path = path.as_ref();

    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => remove_dir_all(path),
        Ok(_) => remove_file(path),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn removes_only_the_named_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.png"), "").unwrap();
        fs::write(dir.path().join("b.png"), "").unwrap();

        remove_artifacts(dir.path(), &["a.png", "missing.png"]).unwrap();

        assert!(!dir.path().join("a.png").exists());
        assert!(dir.path().join("b.png").exists());
    }

    #[test]
    fn never_removes_a_directory_named_as_an_artifact() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("a.png")).unwrap();
        fs::write(dir.path().join("a.png/notes.txt"), "").unwrap();

        assert!(remove_artifacts(dir.path(), &["a.png"]).is_err());
        assert!(dir.path().join("a.png/notes.txt").exists());
    }

    #[test]
    fn removes_shapefiles_and_their_directory_once_empty() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "own/own.shp",
            "own/own.dbf",
            "shared/shared.shp",
            "shared/roads.shp",
        ] {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }

        remove_shapefile_artifacts(dir.path(), &["own/own", "shared/shared", "missing/missing"])
            .unwrap();

        assert!(!dir.path().join("own").exists());
        assert!(!dir.path().join("shared/shared.shp").exists());
        assert!(dir.path().join("shared/roads.shp").exists());
    }

    #[test]
    fn refuses_an_input_that_an_output_would_replace() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("output");
        fs::create_dir(&output).unwrap();
        fs::write(output.join("dem.tif"), "").unwrap();
        fs::write(dir.path().join("dem.tif"), "").unwrap();

        // The same file through another path.
        let input = dir.path().join("output/../output/dem.tif");
        assert!(check_not_an_artifact(&input, &output, &["dem.tif"]).is_err());
        let other = dir.path().join("dem.tif");
        assert!(check_not_an_artifact(&other, &output, &["dem.tif"]).is_ok());
        assert!(check_not_an_artifact(&other, &output, &["extent.txt"]).is_ok());
    }
}
