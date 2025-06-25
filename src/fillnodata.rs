/// Represents a pixel coordinate and value.
#[derive(Clone, Copy)]
struct Point {
    x: usize,
    y: usize,
    value: f32,
}

/// Fills NaNs in a 2D matrix using inverse distance weighting (IDW) and a simple spatial index.
pub fn fill_nodata(input: &Vec<Vec<f32>>, max_dist: usize, iterations: usize) -> Vec<Vec<f32>> {
    let height = input.len();
    let width = input[0].len();
    let mut output = input.clone();

    // Precompute valid points (spatial index)
    let mut valid_points: Vec<Point> = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let val = input[y][x];
            if !val.is_nan() {
                valid_points.push(Point { x, y, value: val });
            }
        }
    }

    let max_dist2 = (max_dist * max_dist) as isize;

    for _ in 0..iterations {
        let mut new_output = output.clone();

        for y in 0..height {
            for x in 0..width {
                if output[y][x].is_nan() {
                    let mut weighted_sum = 0.0;
                    let mut total_weight = 0.0;

                    for &Point { x: px, y: py, value } in &valid_points {
                        let dx = px as isize - x as isize;
                        let dy = py as isize - y as isize;
                        let dist2 = dx * dx + dy * dy;

                        if dist2 > 0 && dist2 <= max_dist2 {
                            let weight = 1.0 / (dist2 as f32);
                            weighted_sum += value * weight;
                            total_weight += weight;
                        }
                    }

                    if total_weight > 0.0 {
                        new_output[y][x] = weighted_sum / total_weight;
                    }
                }
            }
        }

        // Update valid points with newly filled values for next iteration
        valid_points.clear();

        for y in 0..height {
            for x in 0..width {
                let val = new_output[y][x];
                if !val.is_nan() {
                    valid_points.push(Point { x, y, value: val });
                }
            }
        }

        output = new_output;
    }

    output
}
