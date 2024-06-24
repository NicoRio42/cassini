use image::Rgba;

pub const INCH: f32 = 254.0;

pub const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
pub const TRANSPARENT: Rgba<u8> = Rgba([255, 255, 255, 0]);
pub const YELLOW: Rgba<u8> = Rgba([255, 221, 154, 255]);
pub const GREEN_1: Rgba<u8> = Rgba([197, 255, 185, 255]);
pub const GREEN_2: Rgba<u8> = Rgba([139, 255, 116, 255]);
pub const GREEN_3: Rgba<u8> = Rgba([61, 255, 23, 255]);
pub const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);
pub const BROWN: (u8, u8, u8) = (209, 92, 0);
pub const BLUE: (u8, u8, u8) = (0, 255, 255);

pub const CLIFF_THICKNESS: i32 = 4;
pub const CONTOUR_THICKNESS_MILLIMETTER: f32 = 0.14;
pub const MASTER_CONTOUR_THICKNESS_MILLIMETTER: f32 = 0.25;
pub const FORM_CONTOUR_THICKNESS_MILLIMETTER: f32 = 0.1;
pub const FORM_CONTOUR_DASH_LENGTH: f32 = 2.0;
pub const FORM_CONTOUR_DASH_INTERVAL_LENGTH: f32 = 0.2;
pub const INCROSSABLE_BODY_OF_WATER_OUTLINE_WIDTH: f32 = 0.18;
pub const FOOTPATH_WIDTH: f32 = 0.25;
pub const FOOTPATH_DASH_LENGTH: f32 = 2.0;
pub const FOOTPATH_DASH_INTERVAL_LENGTH: f32 = 0.25;
