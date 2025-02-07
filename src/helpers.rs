use std::{
    fs::{read_dir, remove_dir_all, remove_file},
    io,
    path::Path,
};

pub fn remove_dir_content<P: AsRef<Path>>(path: P) -> io::Result<()> {
    for entry in read_dir(path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            remove_dir_all(&path)?;
        } else {
            remove_file(path)?;
        }
    }

    Ok(())
}
