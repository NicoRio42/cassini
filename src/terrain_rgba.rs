pub fn encode_elevation_to_rgba(elevation: f32) -> [u8; 4] {
    let bits = elevation.to_bits(); // converts f32 to u32 bit pattern
    [
        ((bits >> 24) & 0xFF) as u8,
        ((bits >> 16) & 0xFF) as u8,
        ((bits >> 8) & 0xFF) as u8,
        (bits & 0xFF) as u8,
    ]
}

pub fn decode_rgba_to_elevation(rgba: [u8; 4]) -> f32 {
    let bits =
        ((rgba[0] as u32) << 24) | ((rgba[1] as u32) << 16) | ((rgba[2] as u32) << 8) | (rgba[3] as u32);
    f32::from_bits(bits)
}
