pub fn _polyline_to_bezier(
    polyline: &[(f32, f32)],
) -> Vec<((f32, f32), (f32, f32), (f32, f32), (f32, f32))> {
    let mut bezier_points: Vec<((f32, f32), (f32, f32), (f32, f32), (f32, f32))> = Vec::new();

    for i in 0..polyline.len() - 2 {
        let p0 = polyline[i];
        let p1 = polyline[i + 1];
        let p2 = polyline[i + 2];

        let (c1, c2) = _calculate_control_points(p0, p1, p2);

        bezier_points.push((p0, c1, c2, p2))
    }

    bezier_points
}

fn _calculate_control_points(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
) -> ((f32, f32), (f32, f32)) {
    let d01 = ((p1.0 - p0.0).powi(2) + (p1.1 - p0.1).powi(2)).sqrt();
    let d12 = ((p2.0 - p1.0).powi(2) + (p2.1 - p1.1).powi(2)).sqrt();

    let factor = 0.3;
    let c1 = if d01 != 0.0 {
        (
            p1.0 - factor * (p2.0 - p0.0) / d01,
            p1.1 - factor * (p2.1 - p0.1) / d01,
        )
    } else {
        p1
    };
    let c2 = if d12 != 0.0 {
        (
            p1.0 + factor * (p2.0 - p0.0) / d12,
            p1.1 + factor * (p2.1 - p0.1) / d12,
        )
    } else {
        p1
    };

    (c1, c2)
}
