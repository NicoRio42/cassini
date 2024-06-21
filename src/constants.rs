use image::Rgba;

pub const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
pub const TRANSPARENT: Rgba<u8> = Rgba([255, 255, 255, 0]);
pub const YELLOW: Rgba<u8> = Rgba([255, 221, 154, 255]);
pub const GREEN_1: Rgba<u8> = Rgba([197, 255, 185, 255]);
pub const GREEN_2: Rgba<u8> = Rgba([139, 255, 116, 255]);
pub const GREEN_3: Rgba<u8> = Rgba([61, 255, 23, 255]);
pub const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);
pub const BROWN: (u8, u8, u8) = (209, 92, 0);
pub const INCH: f32 = 254.0;
pub const CLIFF_THICKNESS: i32 = 4;
