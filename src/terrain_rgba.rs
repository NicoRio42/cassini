pub fn encode_elevation_to_rgba(elevation: f32) -> [u8; 4] {
    // Clamp elevation to avoid negatives beyond -10000
    // and add offset to ensure positive values
    let offset = 10000.0;
    let scale = 1000.0; // millimeter precision

    // Encode elevation: shift to positive, scale, round
    let encoded = ((elevation.max(-offset) + offset) * scale).round() as u32;

    let r = ((encoded >> 24) & 0xFF) as u8;
    let g = ((encoded >> 16) & 0xFF) as u8;
    let b = ((encoded >> 8) & 0xFF) as u8;
    let a = (encoded & 0xFF) as u8;

    [r, g, b, a]
}

pub fn decode_rgba_to_elevation(rgba: [u8; 4]) -> f32 {
    let offset = 10000.0;
    let scale = 1000.0;

    let encoded =
        ((rgba[0] as u32) << 24) | ((rgba[1] as u32) << 16) | ((rgba[2] as u32) << 8) | (rgba[3] as u32);

    (encoded as f32) / scale - offset
}
