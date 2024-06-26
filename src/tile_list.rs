pub fn get_tile_list_from_extent(
    min_x: u64,
    min_y: u64,
    max_x: u64,
    max_y: u64,
) -> Vec<(u64, u64, u64, u64)> {
    let mut tile_list: Vec<(u64, u64, u64, u64)> = vec![];

    for x in (min_x - 500) / 1000..(max_x + 500) / 1000 {
        for y in (min_y - 500) / 1000..(max_y + 500) / 1000 {
            tile_list.push((
                x * 1000 + 500,
                y * 1000 + 500,
                x * 1000 + 1500,
                y * 1000 + 1500,
            ))
        }
    }

    return tile_list;
}
