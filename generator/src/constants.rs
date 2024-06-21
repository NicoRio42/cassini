use image::Rgba;

pub const VEGETATION_RESOLUTION: u32 = 2;
pub const DEM_RESOLUTION: u32 = 1;
pub const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
pub const TRANSPARENT: Rgba<u8> = Rgba([255, 255, 255, 0]);
pub const YELLOW: Rgba<u8> = Rgba([255, 221, 154, 255]);
pub const GREEN_1: Rgba<u8> = Rgba([197, 255, 185, 255]);
pub const GREEN_2: Rgba<u8> = Rgba([139, 255, 116, 255]);
pub const GREEN_3: Rgba<u8> = Rgba([61, 255, 23, 255]);
pub const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);
pub const BROWN: (u8, u8, u8) = (209, 92, 0);
pub const YELLOW_THRESHOLD: f64 = 700.0;
pub const GREEN_1_THRESHOLD: f64 = 25.0;
pub const GREEN_2_THRESHOLD: f64 = 100.0;
pub const GREEN_3_THRESHOLD: f64 = 150.0;
pub const SLOPE_THRESHOLD: f32 = 40.0;
pub const MIN_X: i32 = 900000;
pub const MIN_Y: i32 = 6440000;
