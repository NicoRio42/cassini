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

pub fn does_segment_intersect_tile(
    segment: ((f32, f32), (f32, f32)),
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
) -> bool {
    let tile_edges = [
        ((min_x, max_y), (max_x, max_y)), // top
        ((max_x, max_y), (max_x, min_y)), // right
        ((max_x, min_y), (min_x, min_y)), // bottom
        ((min_x, min_y), (min_x, max_y)), // left
    ];

    return tile_edges
        .iter()
        .any(|tile_edge| do_intersect(*tile_edge, segment));
}

pub fn does_polyline_intersect_tile(
    polyline: &Vec<(f32, f32)>,
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
) -> bool {
    let min_x = min_x as f32;
    let min_y = min_y as f32;
    let max_x = max_x as f32;
    let max_y = max_y as f32;

    let are_some_points_inside_tile = polyline
        .iter()
        .any(|&(x, y)| x > min_x && y > min_y && x < max_x && y < max_y);

    if are_some_points_inside_tile {
        return true;
    }

    let polyline_length = polyline.len();

    if polyline_length <= 1 {
        return false;
    }

    for index in 0..(polyline_length - 1) {
        let first_point = polyline[index];
        let second_point = polyline[index + 1];

        if does_segment_intersect_tile((first_point, second_point), min_x, min_y, max_x, max_y) {
            return true;
        }
    }

    return false;
}

fn orientation(p: (f32, f32), q: (f32, f32), r: (f32, f32)) -> i32 {
    let val = (q.1 - p.1) * (r.0 - q.0) - (q.0 - p.0) * (r.1 - q.1);
    if val == 0.0 {
        0 // collinear
    } else if val > 0.0 {
        1 // clockwise
    } else {
        2 // counterclockwise
    }
}

fn on_segment(p: (f32, f32), q: (f32, f32), r: (f32, f32)) -> bool {
    q.0 <= f32::max(p.0, r.0)
        && q.0 >= f32::min(p.0, r.0)
        && q.1 <= f32::max(p.1, r.1)
        && q.1 >= f32::min(p.1, r.1)
}

fn do_intersect(segment1: ((f32, f32), (f32, f32)), segment2: ((f32, f32), (f32, f32))) -> bool {
    let (p1, q1) = segment1;
    let (p2, q2) = segment2;
    let o1 = orientation(p1, q1, p2);
    let o2 = orientation(p1, q1, q2);
    let o3 = orientation(p2, q2, p1);
    let o4 = orientation(p2, q2, q1);

    if o1 != o2 && o3 != o4 {
        return true;
    }

    if o1 == 0 && on_segment(p1, p2, q1) {
        return true;
    }
    if o2 == 0 && on_segment(p1, q2, q1) {
        return true;
    }
    if o3 == 0 && on_segment(p2, p1, q2) {
        return true;
    }
    if o4 == 0 && on_segment(p2, q1, q2) {
        return true;
    }

    false
}
