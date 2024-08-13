use rustc_hash::FxHashMap as HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};

use image::RgbaImage;
use imageproc::drawing::draw_line_segment_mut;

use crate::config::Config;
use crate::constants::{FORM_CONTOUR_DASH_INTERVAL_LENGTH, FORM_CONTOUR_DASH_LENGTH};
use crate::{
    constants::{BROWN, PURPLE, TRANSPARENT},
    tile::Tile,
};

pub fn pullautin_render_contours(
    tile: &Tile,
    image_width: u32,
    image_height: u32,
    buffer: i64,
    config: &Config,
) {
    let scalefactor = 1.0;
    let nodepressions = false;
    let formline = 2.0;
    let mut img = RgbaImage::from_pixel(image_width, image_height, TRANSPARENT);
    let mut formlinesteepness: f64 = 0.37;
    let label_depressions = false;

    let mut size: f64 = 0.0;
    let mut xstart: f64 = 0.0;
    let mut ystart: f64 = 0.0;
    let mut steepness: HashMap<(usize, usize), f64> = HashMap::default();
    if formline > 0.0 {
        let path = format!("{}/xyz2.xyz", tmpfolder);
        let xyz_file_in = Path::new(&path);

        if let Ok(lines) = read_lines(xyz_file_in) {
            for (i, line) in lines.enumerate() {
                let ip = line.unwrap_or(String::new());
                let mut parts = ip.split(' ');
                let x: f64 = parts.next().unwrap().parse::<f64>().unwrap();
                let y: f64 = parts.next().unwrap().parse::<f64>().unwrap();

                if i == 0 {
                    xstart = x;
                    ystart = y;
                } else if i == 1 {
                    size = y - ystart;
                } else {
                    break;
                }
            }
        }

        let mut sxmax: usize = usize::MIN;
        let mut symax: usize = usize::MIN;

        let mut xyz: HashMap<(usize, usize), f64> = HashMap::default();

        read_lines_no_alloc(xyz_file_in, |line| {
            let mut parts = line.split(' ');
            let x: f64 = parts.next().unwrap().parse::<f64>().unwrap();
            let y: f64 = parts.next().unwrap().parse::<f64>().unwrap();
            let h: f64 = parts.next().unwrap().parse::<f64>().unwrap();

            let xx = ((x - xstart) / size).floor() as usize;
            let yy = ((y - ystart) / size).floor() as usize;

            xyz.insert((xx, yy), h);

            if sxmax < xx {
                sxmax = xx;
            }
            if symax < yy {
                symax = yy;
            }
        })
        .expect("Unable to read file");

        for i in 6..(sxmax - 7) {
            for j in 6..(symax - 7) {
                let mut det: f64 = 0.0;
                let mut high: f64 = f64::MIN;

                let mut temp =
                    (xyz.get(&(i - 4, j)).unwrap_or(&0.0) - xyz.get(&(i, j)).unwrap_or(&0.0)).abs()
                        / 4.0;
                let temp2 =
                    (xyz.get(&(i, j)).unwrap_or(&0.0) - xyz.get(&(i + 4, j)).unwrap_or(&0.0)).abs()
                        / 4.0;
                let det2 = (xyz.get(&(i, j)).unwrap_or(&0.0)
                    - 0.5
                        * (xyz.get(&(i - 4, j)).unwrap_or(&0.0)
                            + xyz.get(&(i + 4, j)).unwrap_or(&0.0)))
                .abs()
                    - 0.05
                        * (xyz.get(&(i - 4, j)).unwrap_or(&0.0)
                            - xyz.get(&(i + 4, j)).unwrap_or(&0.0))
                        .abs();
                let mut porr = (((xyz.get(&(i - 6, j)).unwrap_or(&0.0)
                    - xyz.get(&(i + 6, j)).unwrap_or(&0.0))
                    / 12.0)
                    .abs()
                    - ((xyz.get(&(i - 3, j)).unwrap_or(&0.0)
                        - xyz.get(&(i + 3, j)).unwrap_or(&0.0))
                        / 6.0)
                        .abs())
                .abs();

                if det2 > det {
                    det = det2;
                }
                if temp2 < temp {
                    temp = temp2;
                }
                if temp > high {
                    high = temp;
                }

                let mut temp =
                    (xyz.get(&(i, j - 4)).unwrap_or(&0.0) - xyz.get(&(i, j)).unwrap_or(&0.0)).abs()
                        / 4.0;
                let temp2 =
                    (xyz.get(&(i, j)).unwrap_or(&0.0) - xyz.get(&(i, j - 4)).unwrap_or(&0.0)).abs()
                        / 4.0;
                let det2 = (xyz.get(&(i, j)).unwrap_or(&0.0)
                    - 0.5
                        * (xyz.get(&(i, j - 4)).unwrap_or(&0.0)
                            + xyz.get(&(i, j + 4)).unwrap_or(&0.0)))
                .abs()
                    - 0.05
                        * (xyz.get(&(i, j - 4)).unwrap_or(&0.0)
                            - xyz.get(&(i, j + 4)).unwrap_or(&0.0))
                        .abs();
                let porr2 = (((xyz.get(&(i, j - 6)).unwrap_or(&0.0)
                    - xyz.get(&(i, j + 6)).unwrap_or(&0.0))
                    / 12.0)
                    .abs()
                    - ((xyz.get(&(i, j - 3)).unwrap_or(&0.0)
                        - xyz.get(&(i, j + 3)).unwrap_or(&0.0))
                        / 6.0)
                        .abs())
                .abs();

                if porr2 > porr {
                    porr = porr2;
                }
                if det2 > det {
                    det = det2;
                }
                if temp2 < temp {
                    temp = temp2;
                }
                if temp > high {
                    high = temp;
                }

                let mut temp = (xyz.get(&(i - 4, j - 4)).unwrap_or(&0.0)
                    - xyz.get(&(i, j)).unwrap_or(&0.0))
                .abs()
                    / 5.6;
                let temp2 = (xyz.get(&(i, j)).unwrap_or(&0.0)
                    - xyz.get(&(i + 4, j + 4)).unwrap_or(&0.0))
                .abs()
                    / 5.6;
                let det2 = (xyz.get(&(i, j)).unwrap_or(&0.0)
                    - 0.5
                        * (xyz.get(&(i - 4, j - 4)).unwrap_or(&0.0)
                            + xyz.get(&(i + 4, j + 4)).unwrap_or(&0.0)))
                .abs()
                    - 0.05
                        * (xyz.get(&(i - 4, j - 4)).unwrap_or(&0.0)
                            - xyz.get(&(i + 4, j + 4)).unwrap_or(&0.0))
                        .abs();
                let porr2 = (((xyz.get(&(i - 6, j - 6)).unwrap_or(&0.0)
                    - xyz.get(&(i + 6, j + 6)).unwrap_or(&0.0))
                    / 17.0)
                    .abs()
                    - ((xyz.get(&(i - 3, j - 3)).unwrap_or(&0.0)
                        - xyz.get(&(i + 3, j + 3)).unwrap_or(&0.0))
                        / 8.5)
                        .abs())
                .abs();

                if porr2 > porr {
                    porr = porr2;
                }
                if det2 > det {
                    det = det2;
                }
                if temp2 < temp {
                    temp = temp2;
                }
                if temp > high {
                    high = temp;
                }

                let mut temp = (xyz.get(&(i - 4, j + 4)).unwrap_or(&0.0)
                    - xyz.get(&(i, j)).unwrap_or(&0.0))
                .abs()
                    / 5.6;
                let temp2 = (xyz.get(&(i, j)).unwrap_or(&0.0)
                    - xyz.get(&(i + 4, j - 4)).unwrap_or(&0.0))
                .abs()
                    / 5.6;
                let det2 = (xyz.get(&(i, j)).unwrap_or(&0.0)
                    - 0.5
                        * (xyz.get(&(i + 4, j - 4)).unwrap_or(&0.0)
                            + xyz.get(&(i - 4, j + 4)).unwrap_or(&0.0)))
                .abs()
                    - 0.05
                        * (xyz.get(&(i + 4, j - 4)).unwrap_or(&0.0)
                            - xyz.get(&(i - 4, j + 4)).unwrap_or(&0.0))
                        .abs();
                let porr2 = (((xyz.get(&(i + 6, j - 6)).unwrap_or(&0.0)
                    - xyz.get(&(i - 6, j + 6)).unwrap_or(&0.0))
                    / 17.0)
                    .abs()
                    - ((xyz.get(&(i + 3, j - 3)).unwrap_or(&0.0)
                        - xyz.get(&(i - 3, j + 3)).unwrap_or(&0.0))
                        / 8.5)
                        .abs())
                .abs();

                if porr2 > porr {
                    porr = porr2;
                }
                if det2 > det {
                    det = det2;
                }
                if temp2 < temp {
                    temp = temp2;
                }
                if temp > high {
                    high = temp;
                }

                let mut val = 12.0 * high / (1.0 + 8.0 * det);
                if porr > 0.25 * 0.67 / (0.3 + formlinesteepness) {
                    val = 0.01;
                }
                if high > val {
                    val = high;
                }
                steepness.insert((i, j), val);
            }
        }
    }

    let input = tile.dir_path.join("contours.dxf");
    let data = fs::read_to_string(input).expect("Can not read input file");
    let data: Vec<&str> = data.split("POLYLINE").collect();

    let mut formline_out = String::new();
    formline_out.push_str(data[0]);

    for (j, rec) in data.iter().enumerate() {
        let mut x = Vec::<f64>::new();
        let mut y = Vec::<f64>::new();
        let mut xline = 0;
        let mut yline = 0;
        let mut layer = "";
        if j > 0 {
            let r = rec.split("VERTEX").collect::<Vec<&str>>();
            let apu = r[1];
            let val = apu.split('\n').collect::<Vec<&str>>();
            layer = val[2].trim();
            for (i, v) in val.iter().enumerate() {
                let vt = v.trim();
                if vt == "10" {
                    xline = i + 1;
                }
                if vt == "20" {
                    yline = i + 1;
                }
            }
            for (i, v) in r.iter().enumerate() {
                if i > 0 {
                    let val = v.trim_end().split('\n').collect::<Vec<&str>>();
                    x.push(
                        (val[xline].trim().parse::<f64>().unwrap() - x0) * 600.0
                            / 254.0
                            / scalefactor,
                    );
                    y.push(
                        (y0 - val[yline].trim().parse::<f64>().unwrap()) * 600.0
                            / 254.0
                            / scalefactor,
                    );
                }
            }
        }
        let mut color = PURPLE; // purple
        if layer.contains("contour") {
            color = BROWN // brown
        }
        if !nodepressions || layer.contains("contour") {
            let mut curvew = 2.0;
            if layer.contains("index") {
                curvew = 3.0;
            }
            if formline > 0.0 {
                if formline == 1.0 {
                    curvew = 2.5
                }
                if layer.contains("intermed") {
                    curvew = 1.5
                }
                if layer.contains("index") {
                    curvew = 3.5
                }
            }

            let mut smallringtest = false;
            let mut help = vec![false; x.len()];
            let mut help2 = vec![false; x.len()];
            if curvew == 1.5 {
                for i in 0..x.len() {
                    help[i] = false;
                    help2[i] = true;
                    let xx = (((x[i] / 600.0 * 254.0 * scalefactor + x0) - xstart) / size).floor();
                    let yy = (((-y[i] / 600.0 * 254.0 * scalefactor + y0) - ystart) / size).floor();
                    if curvew != 1.5
                        || formline == 0.0
                        || steepness.get(&(xx as usize, yy as usize)).unwrap_or(&0.0)
                            < &formlinesteepness
                        || steepness
                            .get(&(xx as usize, yy as usize + 1))
                            .unwrap_or(&0.0)
                            < &formlinesteepness
                        || steepness
                            .get(&(xx as usize + 1, yy as usize))
                            .unwrap_or(&0.0)
                            < &formlinesteepness
                        || steepness
                            .get(&(xx as usize + 1, yy as usize + 1))
                            .unwrap_or(&0.0)
                            < &formlinesteepness
                    {
                        help[i] = true;
                    }
                }
                for i in 5..(x.len() - 6) {
                    let mut apu = 0;
                    for j in (i - 5)..(i + 4) {
                        if help[j] {
                            apu += 1;
                        }
                    }
                    if apu < 5 {
                        help2[i] = false;
                    }
                }
                for i in 0..6 {
                    help2[i] = help2[6]
                }
                for i in (x.len() - 6)..x.len() {
                    help2[i] = help2[x.len() - 7]
                }
                let mut on = 0.0;
                for i in 0..x.len() {
                    if help2[i] {
                        on = config.form_lines.additional_tail_length
                    }
                    if on > 0.0 {
                        help2[i] = true;
                        on -= 1.0;
                    }
                }
                if x.first() == x.last() && y.first() == y.last() && on > 0.0 {
                    let mut i = 0;
                    while i < x.len() && on > 0.0 {
                        help2[i] = true;
                        on -= 1.0;
                        i += 1;
                    }
                }
                let mut on = 0.0;
                for i in 0..x.len() {
                    let ii = x.len() - i - 1;
                    if help2[ii] {
                        on = config.form_lines.additional_tail_length
                    }
                    if on > 0.0 {
                        help2[ii] = true;
                        on -= 1.0;
                    }
                }
                if x.first() == x.last() && y.first() == y.last() && on > 0.0 {
                    let mut i = (x.len() - 1) as i32;
                    while i > -1 && on > 0.0 {
                        help2[i as usize] = true;
                        on -= 1.0;
                        i -= 1;
                    }
                }
                // Let's not break small form line rings
                smallringtest = false;
                if x.first() == x.last() && y.first() == y.last() && x.len() < 122 {
                    for i in 1..x.len() {
                        if help2[i] {
                            smallringtest = true
                        }
                    }
                }
                // Let's draw short gaps together
                if !smallringtest {
                    let mut tester = 1;
                    for i in 1..x.len() {
                        if help2[i] {
                            if tester < i
                                && ((i - tester) as u32) < config.form_lines.min_gap_length as u32
                            {
                                for j in tester..(i + 1) {
                                    help2[j] = true;
                                }
                            }
                            tester = i;
                        }
                    }
                    // Ring handling
                    if x.first() == x.last() && y.first() == y.last() && x.len() < 2 {
                        let mut i = 1;
                        while i < x.len() && !help2[i] {
                            i += 1
                        }
                        let mut j = x.len() - 1;
                        while j > 1 && !help2[i] {
                            j -= 1
                        }
                        if ((x.len() - j + i - 1) as u32) < config.form_lines.min_gap_length as u32
                            && j > i
                        {
                            for k in 0..(i + 1) {
                                help2[k] = true
                            }
                            for k in j..x.len() {
                                help2[k] = true
                            }
                        }
                    }
                }
            }

            let mut linedist = 0.0;
            let mut onegapdone = false;
            let mut gap = 0.0;
            let mut formlinestart = false;

            let f_label;
            if layer.contains("depression") && label_depressions {
                f_label = "formline_depression";
            } else {
                f_label = "formline"
            };

            for i in 1..x.len() {
                if curvew != 1.5 || formline == 0.0 || help2[i] || smallringtest {
                    if formline == 2.0 && !nodepressions && curvew == 1.5 {
                        if !formlinestart {
                            formline_out.push_str(
                                format!("POLYLINE\r\n 66\r\n1\r\n  8\r\n{}\r\n  0\r\n", f_label)
                                    .as_str(),
                            );
                            formlinestart = true;
                        }
                        formline_out.push_str(
                            format!(
                                "VERTEX\r\n  8\r\n{}\r\n 10\r\n{}\r\n 20\r\n{}\r\n  0\r\n",
                                f_label,
                                x[i] / 600.0 * 254.0 * scalefactor + x0,
                                -y[i] / 600.0 * 254.0 * scalefactor + y0
                            )
                            .as_str(),
                        );
                    }
                    if curvew == 1.5 && formline == 2.0 {
                        let step = ((x[i - 1] - x[i]).powi(2) + (y[i - 1] - y[i]).powi(2)).sqrt();
                        if i < 4 {
                            linedist = 0.0
                        }
                        linedist += step;
                        if linedist > FORM_CONTOUR_DASH_LENGTH as f64 && i > 10 && i < x.len() - 11
                        {
                            let mut sum = 0.0;
                            for k in (i - 4)..(i + 6) {
                                sum +=
                                    ((x[k - 1] - x[k]).powi(2) + (y[k - 1] - y[k]).powi(2)).sqrt()
                            }
                            let mut toonearend = false;
                            for k in (i - 10)..(i + 10) {
                                if !help2[k] {
                                    toonearend = true;
                                    break;
                                }
                            }
                            if !toonearend
                                && ((x[i - 5] - x[i + 5]).powi(2) + (y[i - 5] - y[i + 5]).powi(2))
                                    .sqrt()
                                    * 1.138
                                    > sum
                            {
                                linedist = 0.0;
                                gap = FORM_CONTOUR_DASH_INTERVAL_LENGTH as f64;
                                onegapdone = true;
                            }
                        }
                        if !onegapdone && (i < x.len() - 9) && i > 6 {
                            gap = FORM_CONTOUR_DASH_INTERVAL_LENGTH as f64 * 0.82;
                            onegapdone = true;
                            linedist = 0.0
                        }
                        if gap > 0.0 {
                            gap -= step;
                            if gap < 0.0 && onegapdone && step > 0.0 {
                                let mut n = -curvew - 0.5;
                                while n < curvew + 0.5 {
                                    let mut m = -curvew - 0.5;
                                    while m < curvew + 0.5 {
                                        draw_line_segment_mut(
                                            &mut img,
                                            (
                                                ((-x[i - 1] * gap + (step + gap) * x[i]) / step + n)
                                                    as f32,
                                                ((-y[i - 1] * gap + (step + gap) * y[i]) / step + m)
                                                    as f32,
                                            ),
                                            ((x[i] + n) as f32, (y[i] + m) as f32),
                                            color,
                                        );
                                        m += 1.0;
                                    }
                                    n += 1.0;
                                }
                                gap = 0.0;
                            }
                        } else {
                            let mut n = -curvew - 0.5;
                            while n < curvew + 0.5 {
                                let mut m = -curvew - 0.5;
                                while m < curvew + 0.5 {
                                    draw_line_segment_mut(
                                        &mut img,
                                        ((x[i - 1] + n) as f32, (y[i - 1] + m) as f32),
                                        ((x[i] + n) as f32, (y[i] + m) as f32),
                                        color,
                                    );
                                    m += 1.0;
                                }
                                n += 1.0;
                            }
                        }
                    } else {
                        let mut n = -curvew;
                        while n < curvew {
                            let mut m = -curvew;
                            while m < curvew {
                                draw_line_segment_mut(
                                    &mut img,
                                    ((x[i - 1] + n) as f32, (y[i - 1] + m) as f32),
                                    ((x[i] + n) as f32, (y[i] + m) as f32),
                                    color,
                                );
                                m += 1.0;
                            }
                            n += 1.0;
                        }
                    }
                } else if formline == 2.0 && formlinestart && !nodepressions {
                    formline_out.push_str("SEQEND\r\n  0\r\n");
                    formlinestart = false;
                }
            }
            if formline == 2.0 && formlinestart && !nodepressions {
                formline_out.push_str("SEQEND\r\n  0\r\n");
            }
        }
    }
    if formline == 2.0 && !nodepressions {
        formline_out.push_str("ENDSEC\r\n  0\r\nEOF\r\n");
        let output = tile.dir_path.join("formlines.dxf");
        let fp = File::create(output).expect("Unable to create file");
        let mut fp = BufWriter::new(fp);
        fp.write_all(formline_out.as_bytes())
            .expect("Unable to write file");
    }
    // // dotknolls----------
    // let input_filename = &format!("{}/dotknolls.dxf", tmpfolder);
    // let input = Path::new(input_filename);
    // let data = fs::read_to_string(input).expect("Can not read input file");
    // let data = data.split("POINT");

    // for (j, rec) in data.enumerate() {
    //     let mut x: f64 = 0.0;
    //     let mut y: f64 = 0.0;
    //     if j > 0 {
    //         let val = rec.split('\n').collect::<Vec<&str>>();
    //         let layer = val[2].trim();
    //         for (i, v) in val.iter().enumerate() {
    //             let vt = v.trim();
    //             if vt == "10" {
    //                 x = (val[i + 1].trim().parse::<f64>().unwrap() - x0) * 600.0
    //                     / 254.0
    //                     / scalefactor;
    //             }
    //             if vt == "20" {
    //                 y = (y0 - val[i + 1].trim().parse::<f64>().unwrap()) * 600.0
    //                     / 254.0
    //                     / scalefactor;
    //             }
    //         }
    //         if layer == "dotknoll" {
    //             let color = Rgba([166, 85, 43, 255]);

    //             draw_filled_circle_mut(&mut img, (x as i32, y as i32), 7, color)
    //         }
    //     }
    // }
}
