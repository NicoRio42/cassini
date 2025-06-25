/// Fills NaNs in a 2D matrix using inverse distance weighting (IDW) and a simple spatial index.
pub fn fill_nodata(input: &Vec<Vec<f32>>) -> Vec<Vec<f32>> {
    let height = input.len();
    let width = input[0].len();
    let mut output = input.clone();

    for y in 0..height {
        for x in 0..width {
            let val = input[y][x];

            if !val.is_nan() {
                continue;
            }

            let mut weighted_sum = 0.0;
            let mut total_weight = 0.0;

            for x_direction in -1..1 {
                for y_direction in -1..1 {
                    if x_direction == 0 && y_direction == 0 {
                        continue;
                    }

                    let mut neighbor_x: i32 = x as i32;
                    let mut neighbor_y: i32 = y as i32;
                    let mut neighbor_value = f32::NAN;
                    let mut dist_2 = 0;

                    while neighbor_value.is_nan()
                        && neighbor_x > 0
                        && neighbor_x < (width - 1) as i32
                        && neighbor_y > 0
                        && neighbor_y < (height - 1) as i32
                    {
                        neighbor_x = neighbor_x + x_direction;
                        neighbor_y += y_direction;
                        let delta_x = (x as i32).abs_diff(neighbor_x);
                        let delta_y = (y as i32).abs_diff(neighbor_y);
                        dist_2 = delta_x * delta_x + delta_y * delta_y;
                        neighbor_value = input[neighbor_y as usize][neighbor_x as usize];
                    }

                    if neighbor_value.is_nan() {
                        continue;
                    }

                    let weight = 1.0 / dist_2 as f32;
                    weighted_sum += neighbor_value * weight;
                    total_weight += weight;
                }
            }

            output[y][x] = weighted_sum / total_weight;
        }
    }

    output
}
