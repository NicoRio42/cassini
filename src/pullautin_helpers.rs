use std::{
    fs::File,
    io::{self, BufRead},
    path::Path,
};

pub fn read_lines_no_alloc<P>(filename: P, mut line_callback: impl FnMut(&str)) -> io::Result<()>
where
    P: AsRef<Path>,
{
    let file = File::open(filename)?;
    let mut reader = io::BufReader::new(file);

    let mut line_buffer = String::new();
    while reader.read_line(&mut line_buffer)? > 0 {
        // the read line contains the newline delimiter, so we need to trim it off
        let line = line_buffer.trim_end();
        line_callback(line);
        line_buffer.clear();
    }

    Ok(())
}
