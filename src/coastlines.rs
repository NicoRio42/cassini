use log::error;
use shapefile::{
    record::{polygon::GenericPolygon, polyline::GenericPolyline},
    Point, PolygonRing,
};

pub fn get_polygon_with_holes_from_coastlines(
    coastlines: Vec<Vec<(f32, f32)>>,
    islands: Vec<Vec<(f32, f32)>>,
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
) -> (Vec<GenericPolygon<Point>>, Vec<GenericPolyline<Point>>) {
    let mut island_rings: Vec<PolygonRing<Point>> = Vec::new();
    let mut coastlines_edges: Vec<GenericPolyline<Point>> = Vec::new();

    for island in islands {
        let mut points: Vec<Point> = Vec::new();

        for (x, y) in island {
            points.push(Point {
                x: x as f64,
                y: y as f64,
            })
        }

        coastlines_edges.push(GenericPolyline::new(points.clone()));
        island_rings.push(PolygonRing::Inner(points));
    }

    if coastlines.len() != 0 {
        let (merged_linestrings, closed_merged_linestrings) =
            merge_and_filter_linestrings(coastlines, min_x, min_y, max_x, max_y);

        for closed_merged_linestring in closed_merged_linestrings.clone() {
            let mut points: Vec<Point> = Vec::new();

            for (x, y) in closed_merged_linestring {
                points.push(Point::new(x as f64, y as f64))
            }

            coastlines_edges.push(GenericPolyline::new(points.clone()));
            island_rings.push(PolygonRing::Inner(points));
        }

        if merged_linestrings.len() == 0 {
            return (
                vec![generate_square_polygon_with_islands(
                    island_rings,
                    min_x,
                    min_y,
                    max_x,
                    max_y,
                )],
                coastlines_edges,
            );
        }

        for merged_linestring in merged_linestrings.clone() {
            let mut points: Vec<Point> = Vec::new();

            for (x, y) in merged_linestring {
                points.push(Point::new(x as f64, y as f64))
            }

            coastlines_edges.push(GenericPolyline::new(points));
        }

        let merged_and_clipped_coastlines = clip_linestrings(merged_linestrings, min_x, min_y, max_x, max_y);
        let mut polygons: Vec<GenericPolygon<Point>> = Vec::new();
        let mut consumed_coastlines_indexes: Vec<usize> = Vec::new();
        let all_indexes: Vec<usize> = (0..merged_and_clipped_coastlines.len()).collect();

        loop {
            let mut polygon: Vec<(f32, f32)> = Vec::new();
            let mut should_append_current_coastline = true;

            match find_missing(&consumed_coastlines_indexes, &all_indexes) {
                Some(coastline_index) => {
                    let mut current_coastline_index = coastline_index;

                    loop {
                        let coastline = &merged_and_clipped_coastlines[current_coastline_index];

                        if should_append_current_coastline {
                            consumed_coastlines_indexes.push(current_coastline_index);
                            polygon.append(&mut coastline.clone());
                        }

                        let polygon_first_point = polygon[0];

                        // let coastline_last_point = coastline[coastline.len() - 1];
                        let polygon_last_point = polygon[polygon.len() - 1];

                        match get_next_coastline_index_or_tile_vertex(
                            &merged_and_clipped_coastlines,
                            polygon_first_point,
                            polygon_last_point,
                            min_x,
                            min_y,
                            max_x,
                            max_y,
                        ) {
                            CoastlineIndexOrTileVertexOrPolygonStart::TopRigth => {
                                polygon.push((max_x as f32, max_y as f32));
                                should_append_current_coastline = false;
                            }
                            CoastlineIndexOrTileVertexOrPolygonStart::BottomRigth => {
                                polygon.push((max_x as f32, min_y as f32));
                                should_append_current_coastline = false;
                            }
                            CoastlineIndexOrTileVertexOrPolygonStart::BottomLeft => {
                                polygon.push((min_x as f32, min_y as f32));
                                should_append_current_coastline = false;
                            }
                            CoastlineIndexOrTileVertexOrPolygonStart::TopLeft => {
                                polygon.push((min_x as f32, max_y as f32));
                                should_append_current_coastline = false;
                            }
                            CoastlineIndexOrTileVertexOrPolygonStart::CoastlineIndex(
                                next_coastline_index,
                            ) => {
                                current_coastline_index = next_coastline_index;
                                should_append_current_coastline = true;
                            }
                            CoastlineIndexOrTileVertexOrPolygonStart::PolygonStart => {
                                polygon.push(polygon_first_point);
                                break;
                            }
                        }
                    }
                }
                None => {
                    break;
                }
            }

            let mut points: Vec<Point> = Vec::new();

            for (x, y) in polygon {
                points.push(Point::new(x as f64, y as f64))
            }

            let mut rings: Vec<PolygonRing<Point>> = vec![PolygonRing::Outer(points)];
            let mut island_rings_copy = island_rings.clone();
            rings.append(&mut island_rings_copy);
            let generic_polygon = GenericPolygon::with_rings(rings);
            polygons.push(generic_polygon);
        }

        return (polygons, coastlines_edges);
    }

    return (
        vec![generate_square_polygon_with_islands(
            island_rings,
            min_x,
            min_y,
            max_x,
            max_y,
        )],
        coastlines_edges,
    );
}

fn generate_square_polygon_with_islands(
    island_rings: Vec<PolygonRing<Point>>,
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
) -> GenericPolygon<Point> {
    let mut rings: Vec<PolygonRing<Point>> = vec![PolygonRing::Outer(vec![
        Point::new(min_x as f64, min_y as f64),
        Point::new(min_x as f64, max_y as f64),
        Point::new(max_x as f64, max_y as f64),
        Point::new(max_x as f64, min_y as f64),
        Point::new(min_x as f64, min_y as f64),
    ])];

    let mut island_rings_copy = island_rings.clone();
    rings.append(&mut island_rings_copy);

    return GenericPolygon::with_rings(rings);
}

fn find_missing(first: &[usize], second: &[usize]) -> Option<usize> {
    second.iter().find(|&&x| !first.contains(&x)).copied()
}

fn merge_and_filter_linestrings(
    mut linestrings: Vec<Vec<(f32, f32)>>,
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
) -> (Vec<Vec<(f32, f32)>>, Vec<Vec<(f32, f32)>>) {
    loop {
        let len = linestrings.len();
        let mut merged = false;

        'outer: for i in 0..len {
            for j in 0..len {
                if i != j {
                    match try_to_merge_two_linestrings(&linestrings[i], &linestrings[j]) {
                        Some(merged_linestringes) => {
                            linestrings.remove(i);
                            linestrings.remove(j - 1);
                            linestrings.push(merged_linestringes);
                            merged = true;
                            break 'outer;
                        }
                        None => continue,
                    }
                }
            }
        }

        if !merged {
            break;
        }
    }

    let mut merged_linestrings: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut closed_merged_linestrings: Vec<Vec<(f32, f32)>> = Vec::new();

    for linestring in linestrings {
        let is_linestring_outside_tile = linestring
            .iter()
            .all(|&(x, y)| x < min_x as f32 || y < min_y as f32 || x > max_x as f32 || y > max_y as f32);

        if is_linestring_outside_tile {
            continue;
        }

        let first_point = linestring[0];
        let last_point = linestring[linestring.len() - 1];
        let is_closed_coastline = first_point.0 == last_point.0 && first_point.1 == last_point.1;

        if is_closed_coastline {
            closed_merged_linestrings.push(linestring);
        } else {
            merged_linestrings.push(linestring)
        }
    }

    (merged_linestrings, closed_merged_linestrings)
}

fn try_to_merge_two_linestrings(
    linestrings_1: &Vec<(f32, f32)>,
    linestrings_2: &Vec<(f32, f32)>,
) -> Option<Vec<(f32, f32)>> {
    let linestring_1_first_point = linestrings_1[0];
    let linestring_2_first_point = linestrings_2[0];
    let linestring_1_last_point = linestrings_1[linestrings_1.len() - 1];
    let linestring_2_last_point = linestrings_2[linestrings_2.len() - 1];

    if linestring_1_first_point.0 == linestring_2_last_point.0
        && linestring_1_first_point.1 == linestring_2_last_point.1
    {
        let mut merged = linestrings_2.clone();
        merged.append(&mut (linestrings_1.clone()));
        return Some(merged);
    }

    if linestring_2_first_point.0 == linestring_1_last_point.0
        && linestring_2_first_point.1 == linestring_1_last_point.1
    {
        let mut merged = linestrings_1.clone();
        merged.append(&mut (linestrings_2.clone()));
        return Some(merged);
    }

    return None;
}

fn clip_linestrings(
    linestrings: Vec<Vec<(f32, f32)>>,
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
) -> Vec<Vec<(f32, f32)>> {
    let mut clipped_and_merged_linestrings: Vec<Vec<(f32, f32)>> = Vec::new();

    for linestring in linestrings {
        let mut first_point_outside_tile_index = 0;
        let mut has_first_point_outside_tile_been_set = false;
        let mut last_point_outside_tile_index = linestring.len() - 1;

        if is_inside_tile(
            linestring[first_point_outside_tile_index],
            min_x,
            min_y,
            max_x,
            max_y,
        ) {
            error!("First point of coastline is inside tile.")
        }

        if is_inside_tile(
            linestring[last_point_outside_tile_index],
            min_x,
            min_y,
            max_x,
            max_y,
        ) {
            error!("Last point of coastline is inside tile.")
        }

        for index in 0..(linestring.len() - 1) {
            let previous_point = linestring[index];
            let next_point: (f32, f32) = linestring[index + 1];

            let is_previous_point_inside = is_inside_tile(previous_point, min_x, min_y, max_x, max_y);

            let is_next_point_inside = is_inside_tile(next_point, min_x, min_y, max_x, max_y);

            if !is_previous_point_inside && is_next_point_inside && !has_first_point_outside_tile_been_set {
                first_point_outside_tile_index = index;
                has_first_point_outside_tile_been_set = true;
            }

            if is_previous_point_inside && !is_next_point_inside {
                last_point_outside_tile_index = index;
            }
        }

        let truncated_first_point = get_intersection_between_segment_and_tile(
            linestring[first_point_outside_tile_index],
            linestring[first_point_outside_tile_index + 1],
            min_x,
            min_y,
            max_x,
            max_y,
        )
        .expect("It should intersect");

        let truncated_last_point = get_intersection_between_segment_and_tile(
            linestring[last_point_outside_tile_index],
            linestring[last_point_outside_tile_index + 1],
            min_x,
            min_y,
            max_x,
            max_y,
        )
        .expect("It should intersect");

        let mut clipped_linestring: Vec<(f32, f32)> = Vec::new();
        clipped_linestring.push(truncated_first_point);

        clipped_linestring.extend_from_slice(
            &linestring[(first_point_outside_tile_index + 1)..last_point_outside_tile_index],
        );

        clipped_linestring.push(truncated_last_point);

        clipped_and_merged_linestrings.push(clipped_linestring)
    }

    return clipped_and_merged_linestrings;
}

enum CoastlineIndexOrTileVertexOrPolygonStart {
    TopRigth,
    BottomRigth,
    BottomLeft,
    TopLeft,
    CoastlineIndex(usize),
    PolygonStart,
}

fn get_next_coastline_index_or_tile_vertex(
    remaining_coastlines: &Vec<Vec<(f32, f32)>>,
    first_point_of_polygon: (f32, f32),
    last_point_of_last_coastline: (f32, f32),
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
) -> CoastlineIndexOrTileVertexOrPolygonStart {
    // Last point on top edge
    if last_point_of_last_coastline.1 == max_y as f32 && last_point_of_last_coastline.0 != max_x as f32 {
        let mut next_coastline_index_or_tile_vertex = CoastlineIndexOrTileVertexOrPolygonStart::TopRigth;
        let mut distance_to_next_coastline = f32::MAX;

        for (index, next_coastline) in remaining_coastlines.iter().enumerate() {
            let does_next_coastline_start_on_top_edge = next_coastline[0].1 == max_y as f32;

            let does_next_coastline_start_right_to_previous_coastline_end =
                next_coastline[0].0 > last_point_of_last_coastline.0;

            if !does_next_coastline_start_on_top_edge
                || !does_next_coastline_start_right_to_previous_coastline_end
            {
                continue;
            }

            let distance = next_coastline[0].0 - last_point_of_last_coastline.0;

            if distance < distance_to_next_coastline {
                next_coastline_index_or_tile_vertex =
                    CoastlineIndexOrTileVertexOrPolygonStart::CoastlineIndex(index);

                distance_to_next_coastline = distance;
            }
        }

        if first_point_of_polygon.0 - last_point_of_last_coastline.0 == distance_to_next_coastline {
            return CoastlineIndexOrTileVertexOrPolygonStart::PolygonStart;
        }

        return next_coastline_index_or_tile_vertex;
    }

    // Last point on right edge
    if last_point_of_last_coastline.0 == max_x as f32 && last_point_of_last_coastline.1 != min_y as f32 {
        let mut next_coastline_index_or_tile_vertex = CoastlineIndexOrTileVertexOrPolygonStart::BottomRigth;
        let mut distance_to_next_coastline = f32::MAX;

        for (index, next_coastline) in remaining_coastlines.iter().enumerate() {
            let does_next_coastline_start_on_right_edge = next_coastline[0].0 == max_x as f32;

            let does_next_coastline_start_below_previous_coastline_end =
                next_coastline[0].1 < last_point_of_last_coastline.1;

            if !does_next_coastline_start_on_right_edge
                || !does_next_coastline_start_below_previous_coastline_end
            {
                continue;
            }

            let distance = last_point_of_last_coastline.1 - next_coastline[0].1;

            if distance < distance_to_next_coastline {
                next_coastline_index_or_tile_vertex =
                    CoastlineIndexOrTileVertexOrPolygonStart::CoastlineIndex(index);

                distance_to_next_coastline = distance;
            }
        }

        if last_point_of_last_coastline.1 - first_point_of_polygon.1 == distance_to_next_coastline {
            return CoastlineIndexOrTileVertexOrPolygonStart::PolygonStart;
        }

        return next_coastline_index_or_tile_vertex;
    }

    // Last point on bottom edge
    if last_point_of_last_coastline.1 == min_y as f32 && last_point_of_last_coastline.0 != min_x as f32 {
        let mut next_coastline_index_or_tile_vertex = CoastlineIndexOrTileVertexOrPolygonStart::BottomLeft;
        let mut distance_to_next_coastline = f32::MAX;

        for (index, next_coastline) in remaining_coastlines.iter().enumerate() {
            let does_next_coastline_start_on_bottom_edge = next_coastline[0].1 == min_y as f32;

            let does_next_coastline_start_left_to_previous_coastline_end =
                next_coastline[0].0 < last_point_of_last_coastline.0;

            if !does_next_coastline_start_on_bottom_edge
                || !does_next_coastline_start_left_to_previous_coastline_end
            {
                continue;
            }

            let distance = last_point_of_last_coastline.0 - next_coastline[0].0;

            if distance < distance_to_next_coastline {
                next_coastline_index_or_tile_vertex =
                    CoastlineIndexOrTileVertexOrPolygonStart::CoastlineIndex(index);

                distance_to_next_coastline = distance;
            }
        }

        if last_point_of_last_coastline.0 - first_point_of_polygon.0 == distance_to_next_coastline {
            return CoastlineIndexOrTileVertexOrPolygonStart::PolygonStart;
        }

        return next_coastline_index_or_tile_vertex;
    }

    // Last point on left edge
    if last_point_of_last_coastline.0 == min_x as f32 && last_point_of_last_coastline.1 != max_y as f32 {
        let mut next_coastline_index_or_tile_vertex = CoastlineIndexOrTileVertexOrPolygonStart::TopLeft;
        let mut distance_to_next_coastline = f32::MAX;

        for (index, next_coastline) in remaining_coastlines.iter().enumerate() {
            let does_next_coastline_start_on_left_edge = next_coastline[0].0 == min_x as f32;

            let does_next_coastline_start_above_previous_coastline_end =
                next_coastline[0].1 > last_point_of_last_coastline.1;

            if !does_next_coastline_start_on_left_edge
                || !does_next_coastline_start_above_previous_coastline_end
            {
                continue;
            }

            let distance = next_coastline[0].1 - last_point_of_last_coastline.1;

            if distance < distance_to_next_coastline {
                next_coastline_index_or_tile_vertex =
                    CoastlineIndexOrTileVertexOrPolygonStart::CoastlineIndex(index);

                distance_to_next_coastline = distance;
            }
        }

        if first_point_of_polygon.1 - last_point_of_last_coastline.1 == distance_to_next_coastline {
            return CoastlineIndexOrTileVertexOrPolygonStart::PolygonStart;
        }

        return next_coastline_index_or_tile_vertex;
    }

    return CoastlineIndexOrTileVertexOrPolygonStart::PolygonStart;
}

fn is_inside_tile(point: (f32, f32), min_x: i64, min_y: i64, max_x: i64, max_y: i64) -> bool {
    return point.0 >= min_x as f32
        && point.0 <= max_x as f32
        && point.1 >= min_y as f32
        && point.1 <= max_y as f32;
}

fn get_intersection_between_segment_and_tile(
    p1: (f32, f32),
    p2: (f32, f32),
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
) -> Option<(f32, f32)> {
    let min_x = min_x as f32;
    let min_y = min_y as f32;
    let max_x = max_x as f32;
    let max_y = max_y as f32;

    // Top
    get_segments_intersection(p1, p2, (min_x, max_y), (max_x, max_y))
        // Right
        .or(get_segments_intersection(p1, p2, (max_x, max_y), (max_x, min_y)))
        // Bottom
        .or(get_segments_intersection(p1, p2, (min_x, min_y), (max_x, min_y)))
        // Left
        .or(get_segments_intersection(p1, p2, (min_x, min_y), (min_x, max_y)))
}

fn get_segments_intersection(
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    p4: (f32, f32),
) -> Option<(f32, f32)> {
    let denominator = (p1.0 - p2.0) * (p3.1 - p4.1) - (p1.1 - p2.1) * (p3.0 - p4.0);

    if denominator.abs() < f32::EPSILON {
        return None; // The lines are parallel or coincident
    }

    let t = ((p1.0 - p3.0) * (p3.1 - p4.1) - (p1.1 - p3.1) * (p3.0 - p4.0)) / denominator;
    let u = ((p1.0 - p3.0) * (p1.1 - p2.1) - (p1.1 - p3.1) * (p1.0 - p2.0)) / denominator;

    if t >= 0.0 && t <= 1.0 && u >= 0.0 && u <= 1.0 {
        let intersection = (p1.0 + t * (p2.0 - p1.0), p1.1 + t * (p2.1 - p1.1));

        Some(intersection)
    } else {
        None // The intersection does not lie within the segment bounds
    }
}
