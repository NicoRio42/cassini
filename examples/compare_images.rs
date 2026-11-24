//! Compares two rendered images pixel by pixel, or every PNG and WebP image
//! of two output directories.
//!
//! ```text
//! cargo run --release --example compare_images -- a.png b.png [--diff diff.png]
//! cargo run --release --example compare_images -- out-a out-b [--diff diff-dir]
//! ```
//!
//! With directories, the images are matched by relative path, an image
//! missing on either side counting as a difference, and `--diff` is a
//! directory receiving a diff image per differing pair, named after the image
//! with `.png` appended unless it is a PNG. In a diff image, the differing
//! pixels are red over a faded copy of `a`. Exits with status 1 if any image
//! differs, and 2 if any image cannot be read.

use clap::Parser;
use image::{Rgba, RgbaImage};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Parser)]
struct Args {
    a: PathBuf,
    b: PathBuf,

    /// Where to write the diff image (a directory when comparing directories).
    #[arg(long, value_name = "PATH")]
    diff: Option<PathBuf>,

    /// The largest channel difference that still counts as equal.
    #[arg(long, default_value_t = 0)]
    tolerance: u8,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let result = if args.a.is_dir() {
        compare_directories(&args)
    } else {
        compare_files(&args.a, &args.b, args.diff.as_deref(), args.tolerance)
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

fn compare_directories(args: &Args) -> Result<bool, String> {
    let (mut in_a, mut in_b) = (BTreeSet::new(), BTreeSet::new());
    find_images(&args.a, &args.a, &mut in_a)?;
    find_images(&args.b, &args.b, &mut in_b)?;
    if in_a.is_empty() {
        return Err(format!("no PNG or WebP image in `{}`", args.a.display()));
    }

    let (mut all_equal, mut errors) = (true, 0usize);
    for relative in in_a.union(&in_b) {
        print!("{}: ", relative.display());
        let missing_from = match (in_a.contains(relative), in_b.contains(relative)) {
            (true, false) => Some(&args.b),
            (false, true) => Some(&args.a),
            _ => None,
        };
        if let Some(dir) = missing_from {
            println!("MISSING in `{}`", dir.display());
            all_equal = false;
            continue;
        }
        let diff = args.diff.as_ref().map(|dir| diff_path(dir, relative));
        let (a, b) = (args.a.join(relative), args.b.join(relative));
        match compare_files(&a, &b, diff.as_deref(), args.tolerance) {
            Ok(equal) => all_equal &= equal,
            Err(message) => {
                println!("ERROR: {message}");
                errors += 1;
            }
        }
    }
    if errors > 0 {
        return Err(format!("could not compare {errors} image(s)"));
    }
    Ok(all_equal)
}

/// The diff image of `relative` in `dir`, so that `x.png` and `x.webp` do not
/// share one.
fn diff_path(dir: &Path, relative: &Path) -> PathBuf {
    if relative.extension().is_some_and(|extension| extension == "png") {
        return dir.join(relative);
    }
    let mut name = OsString::from(relative.as_os_str());
    name.push(".png");
    dir.join(name)
}

/// Adds the paths relative to `root` of the images under `dir` to `images`.
fn find_images(root: &Path, dir: &Path, images: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("could not read `{}`: {error}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            find_images(root, &path, images)?;
        } else if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("png" | "webp")
        ) {
            images.insert(path.strip_prefix(root).unwrap().to_path_buf());
        }
    }
    Ok(())
}

/// Prints the comparison and returns whether the images are equal.
fn compare_files(a: &Path, b: &Path, diff: Option<&Path>, tolerance: u8) -> Result<bool, String> {
    let open = |path: &Path| {
        image::open(path)
            .map(|image| image.to_rgba8())
            .map_err(|error| format!("could not read `{}`: {error}", path.display()))
    };
    let (image_a, image_b) = (open(a)?, open(b)?);
    if image_a.dimensions() != image_b.dimensions() {
        println!(
            "DIFFERENT sizes: {:?} and {:?}",
            image_a.dimensions(),
            image_b.dimensions()
        );
        return Ok(false);
    }

    let mut differing = 0usize;
    let mut max_channel_difference = 0u8;
    let mut diff_image = diff.map(|_| RgbaImage::new(image_a.width(), image_a.height()));
    for (index, (pixel_a, pixel_b)) in image_a.pixels().zip(image_b.pixels()).enumerate() {
        let channel_difference = pixel_a
            .0
            .iter()
            .zip(pixel_b.0)
            .map(|(&channel_a, channel_b)| channel_a.abs_diff(channel_b))
            .max()
            .unwrap();
        max_channel_difference = max_channel_difference.max(channel_difference);
        let differs = channel_difference > tolerance;
        differing += differs as usize;

        if let Some(diff_image) = &mut diff_image {
            let x = index as u32 % image_a.width();
            let y = index as u32 / image_a.width();
            let pixel = if differs {
                Rgba([255, 0, 0, 255])
            } else {
                let [red, green, blue, alpha] = pixel_a.0;
                let luma = (red as u32 * 299 + green as u32 * 587 + blue as u32 * 114) / 1000;
                let luma = luma * alpha as u32 / 255 + (255 - alpha as u32);
                let faded = (191 + luma / 4) as u8;
                Rgba([faded, faded, faded, 255])
            };
            diff_image.put_pixel(x, y, pixel);
        }
    }

    let total = image_a.width() as usize * image_a.height() as usize;
    println!(
        "{} x {} px, {differing} differing ({:.4}%), max channel difference {max_channel_difference}",
        image_a.width(),
        image_a.height(),
        100.0 * differing as f64 / total as f64
    );
    if let (Some(path), Some(diff_image)) = (diff, diff_image) {
        if differing > 0 {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            diff_image
                .save(path)
                .map_err(|error| format!("could not write `{}`: {error}", path.display()))?;
            println!("   diff image: {}", path.display());
        }
    }
    Ok(differing == 0)
}
