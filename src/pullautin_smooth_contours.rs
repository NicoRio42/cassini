use std::error::Error;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use tiff::decoder::{Decoder, DecodingResult};

use rustc_hash::FxHashMap as HashMap;

use crate::tile::Tile;

pub fn smoothjoin(tile: &Tile, buffer: i64) -> Result<(), Box<dyn Error>> {
    println!("Smooth curves...");
    let scalefactor: f64 = 1.0;
    let inidotknolls: f64 = 0.8;
    let smoothing: f64 = 0.7;
    let curviness: f64 = 1.1;
    let mut indexcontours: f64 = 12.5;
    let formline: f64 = 2.0;
    let contour_interval: f64 = 5.0;
    let depression_length: usize = 181;
    let halfinterval = contour_interval / 2.0 * scalefactor;
    if formline > 0.0 {
        indexcontours = 5.0 * contour_interval;
    }
    let interval = halfinterval;

    let dem_path = tile.dir_path.join("dem-low-resolution-with-buffer.tif");
    let dem_tif_file = File::open(dem_path).expect("Cannot find low resolution dem tif image!");

    let mut dem_img_decoder = Decoder::new(dem_tif_file).expect("Cannot create decoder");
    dem_img_decoder = dem_img_decoder.with_limits(tiff::decoder::Limits::unlimited());

    let (dem_width, dem_height) = dem_img_decoder.dimensions().unwrap();
    let DecodingResult::F64(image_data) = dem_img_decoder.read_image().unwrap() else {
        panic!("Cannot read band data")
    };

    // let path = format!("{}/xyz_knolls.xyz", tmpfolder);
    // let xyz_file_in = Path::new(&path);
    let size: f64 = 2.0;
    let xstart: f64 = (tile.min_x - buffer) as f64;
    let ystart: f64 = (tile.min_y - buffer) as f64;

    // if let Ok(lines) = read_lines(xyz_file_in) {
    //     for (i, line) in lines.enumerate() {
    //         let ip = line.unwrap_or(String::new());
    //         let mut parts = ip.split(' ');
    //         let x: f64 = parts.next().unwrap().parse::<f64>().unwrap();
    //         let y: f64 = parts.next().unwrap().parse::<f64>().unwrap();
    //         if i == 0 {
    //             xstart = x;
    //             ystart = y;
    //         } else if i == 1 {
    //             size = y - ystart;
    //         } else {
    //             break;
    //         }
    //     }
    // }

    let xmax: u64 = (tile.max_x + buffer - xstart as i64) as u64;
    let ymax: u64 = (tile.max_y + buffer - ystart as i64) as u64;
    let mut xyz: HashMap<(u64, u64), f64> = HashMap::default();

    for index in 0..image_data.len() {
        let x = (index % usize::try_from(dem_width).unwrap()) as f64;
        let y = (usize::try_from(dem_height).unwrap()
            - index / usize::try_from(dem_height).unwrap()) as f64;
        let h = image_data[index] as f64;

        let xx = x.floor() as u64;
        let yy = y.floor() as u64;

        xyz.insert((xx, yy), h);
    }

    // read_lines_no_alloc(xyz_file_in, |line| {
    //     let mut parts = line.split(' ');
    //     let x: f64 = parts.next().unwrap().parse::<f64>().unwrap();
    //     let y: f64 = parts.next().unwrap().parse::<f64>().unwrap();
    //     let h: f64 = parts.next().unwrap().parse::<f64>().unwrap();

    //     let xx = ((x - xstart) / size).floor() as u64;
    //     let yy = ((y - ystart) / size).floor() as u64;

    //     xyz.insert((xx, yy), h);

    //     if xmax < xx {
    //         xmax = xx;
    //     }
    //     if ymax < yy {
    //         ymax = yy;
    //     }
    // })
    // .expect("error reading xyz file");

    let mut steepness = vec![vec![f64::NAN; (ymax + 1) as usize]; (xmax + 1) as usize];
    for i in 1..xmax {
        for j in 1..ymax {
            let mut low: f64 = f64::MAX;
            let mut high: f64 = f64::MIN;
            for ii in i - 1..i + 2 {
                for jj in j - 1..j + 2 {
                    let tmp = *xyz.get(&(ii, jj)).unwrap_or(&0.0);
                    if tmp < low {
                        low = tmp;
                    }
                    if tmp > high {
                        high = tmp;
                    }
                }
            }
            steepness[i as usize][j as usize] = high - low;
        }
    }

    let input = tile.dir_path.join("contours-raw.dxf");
    let data = fs::read_to_string(input).expect("Can not read input file");
    let data: Vec<&str> = data.split("POLYLINE").collect();
    let mut dxfheadtmp = data[0];
    dxfheadtmp = dxfheadtmp.split("ENDSEC").collect::<Vec<&str>>()[0];
    dxfheadtmp = dxfheadtmp.split("HEADER").collect::<Vec<&str>>()[1];
    let dxfhead = &format!("HEADER{}ENDSEC", dxfheadtmp);
    let mut out = String::new();
    out.push_str("  0\r\nSECTION\r\n  2\r\n");
    out.push_str(dxfhead);
    out.push_str("\r\n  0\r\nSECTION\r\n  2\r\nENTITIES\r\n  0\r\n");

    let depr_output = tile.dir_path.join("depressions.txt");
    let depr_fp = File::create(depr_output).expect("Unable to create file");
    let mut depr_fp = BufWriter::new(depr_fp);

    let dotknoll_output = tile.dir_path.join("dotknolls.txt");
    let dotknoll_fp = File::create(dotknoll_output).expect("Unable to create file");
    let mut dotknoll_fp = BufWriter::new(dotknoll_fp);

    let knollhead_output = tile.dir_path.join("knollheads.txt");
    let knollhead_fp = File::create(knollhead_output).expect("Unable to create file");
    let mut knollhead_fp = BufWriter::new(knollhead_fp);

    let mut heads1: HashMap<String, usize> = HashMap::default();
    let mut heads2: HashMap<String, usize> = HashMap::default();
    let mut heads = Vec::<String>::new();
    let mut tails = Vec::<String>::new();
    let mut el_x = Vec::<Vec<f64>>::new();
    let mut el_y = Vec::<Vec<f64>>::new();
    el_x.push(vec![]);
    el_y.push(vec![]);
    heads.push(String::from("-"));
    tails.push(String::from("-"));

    for (j, rec) in data.iter().enumerate() {
        let mut x = Vec::<f64>::new();
        let mut y = Vec::<f64>::new();
        let mut xline = 0;
        let mut yline = 0;
        if j > 0 {
            let r = rec.split("VERTEX").collect::<Vec<&str>>();
            let apu = r[1];
            let val = apu.split('\n').collect::<Vec<&str>>();
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
                    x.push(val[xline].trim().parse::<f64>().unwrap());
                    y.push(val[yline].trim().parse::<f64>().unwrap());
                }
            }
            let x0 = x.first().unwrap();
            let xl = x.last().unwrap();
            let y0 = y.first().unwrap();
            let yl = y.last().unwrap();
            let head = format!("{}x{}", x0, y0);
            let tail = format!("{}x{}", xl, yl);

            heads.push(head);
            tails.push(tail);

            let head = format!("{}x{}", x0, y0);
            let tail = format!("{}x{}", xl, yl);
            el_x.push(x);
            el_y.push(y);
            if *heads1.get(&head).unwrap_or(&0) == 0 {
                heads1.insert(head, j);
            } else {
                heads2.insert(head, j);
            }
            if *heads1.get(&tail).unwrap_or(&0) == 0 {
                heads1.insert(tail, j);
            } else {
                heads2.insert(tail, j);
            }
        }
    }

    for l in 0..data.len() {
        let mut to_join = 0;
        if !el_x[l].is_empty() {
            let mut end_loop = false;
            while !end_loop {
                let tmp = *heads1.get(&heads[l]).unwrap_or(&0);
                if tmp != 0 && tmp != l && !el_x[tmp].is_empty() {
                    to_join = tmp;
                } else {
                    let tmp = *heads2.get(&heads[l]).unwrap_or(&0);
                    if tmp != 0 && tmp != l && !el_x[tmp].is_empty() {
                        to_join = tmp;
                    } else {
                        let tmp = *heads2.get(&tails[l]).unwrap_or(&0);
                        if tmp != 0 && tmp != l && !el_x[tmp].is_empty() {
                            to_join = tmp;
                        } else {
                            let tmp = *heads1.get(&tails[l]).unwrap_or(&0);
                            if tmp != 0 && tmp != l && !el_x[tmp].is_empty() {
                                to_join = tmp;
                            } else {
                                end_loop = true;
                            }
                        }
                    }
                }
                if !end_loop {
                    if tails[l] == heads[to_join] {
                        let tmp = tails[l].to_string();
                        heads2.insert(tmp, 0);
                        let tmp = tails[l].to_string();
                        heads1.insert(tmp, 0);
                        let mut to_append = el_x[to_join].to_vec();
                        el_x[l].append(&mut to_append);
                        let mut to_append = el_y[to_join].to_vec();
                        el_y[l].append(&mut to_append);
                        let tmp = tails[to_join].to_string();
                        tails[l] = tmp;
                        el_x[to_join].clear();
                    } else if tails[l] == tails[to_join] {
                        let tmp = tails[l].to_string();
                        heads2.insert(tmp, 0);
                        let tmp = tails[l].to_string();
                        heads1.insert(tmp, 0);
                        let mut to_append = el_x[to_join].to_vec();
                        to_append.reverse();
                        el_x[l].append(&mut to_append);
                        let mut to_append = el_y[to_join].to_vec();
                        to_append.reverse();
                        el_y[l].append(&mut to_append);
                        let tmp = heads[to_join].to_string();
                        tails[l] = tmp;
                        el_x[to_join].clear();
                    } else if heads[l] == tails[to_join] {
                        let tmp = heads[l].to_string();
                        heads2.insert(tmp, 0);
                        let tmp = heads[l].to_string();
                        heads1.insert(tmp, 0);
                        let to_append = el_x[to_join].to_vec();
                        el_x[l].splice(0..0, to_append);
                        let to_append = el_y[to_join].to_vec();
                        el_y[l].splice(0..0, to_append);
                        let tmp = heads[to_join].to_string();
                        heads[l] = tmp;
                        el_x[to_join].clear();
                    } else if heads[l] == heads[to_join] {
                        let tmp = heads[l].to_string();
                        heads2.insert(tmp, 0);
                        let tmp = heads[l].to_string();
                        heads1.insert(tmp, 0);
                        let mut to_append = el_x[to_join].to_vec();
                        to_append.reverse();
                        el_x[l].splice(0..0, to_append);
                        let mut to_append = el_y[to_join].to_vec();
                        to_append.reverse();
                        el_y[l].splice(0..0, to_append);
                        let tmp = tails[to_join].to_string();
                        heads[l] = tmp;
                        el_x[to_join].clear();
                    }
                }
            }
        }
    }
    for l in 0..data.len() {
        let mut el_x_len = el_x[l].len();
        if el_x_len > 0 {
            let mut skip = false;
            let mut depression = 1;
            if el_x_len < 3 {
                skip = true;
                el_x[l].clear();
            }
            let mut h = f64::NAN;
            if !skip {
                let mut mm: isize = (((el_x_len - 1) as f64) / 3.0).floor() as isize - 1;
                if mm < 0 {
                    mm = 0;
                }
                let mut m = mm as usize;
                while m < el_x_len {
                    let xm = el_x[l][m];
                    let ym = el_y[l][m];
                    if (xm - xstart) / size == ((xm - xstart) / size).floor() {
                        let xx = ((xm - xstart) / size).floor() as u64;
                        let yy = ((ym - ystart) / size).floor() as u64;
                        let h1 = *xyz.get(&(xx, yy)).unwrap_or(&0.0);
                        let h2 = *xyz.get(&(xx, yy + 1)).unwrap_or(&0.0);
                        let h3 = h1 * (yy as f64 + 1.0 - (ym - ystart) / size)
                            + h2 * ((ym - ystart) / size - yy as f64);
                        h = (h3 / interval + 0.5).floor() * interval;
                        m += el_x_len;
                    } else if m < el_x_len - 1
                        && (el_y[l][m] - ystart) / size == ((el_y[l][m] - ystart) / size).floor()
                    {
                        let xx = ((xm - xstart) / size).floor() as u64;
                        let yy = ((ym - ystart) / size).floor() as u64;
                        let h1 = *xyz.get(&(xx, yy)).unwrap_or(&0.0);
                        let h2 = *xyz.get(&(xx + 1, yy)).unwrap_or(&0.0);
                        let h3 = h1 * (xx as f64 + 1.0 - (xm - xstart) / size)
                            + h2 * ((xm - xstart) / size - xx as f64);
                        h = (h3 / interval + 0.5).floor() * interval;
                        m += el_x_len;
                    } else {
                        m += 1;
                    }
                }
            }
            if !skip
                && el_x_len < depression_length
                && el_x[l].first() == el_x[l].last()
                && el_y[l].first() == el_y[l].last()
            {
                let mut mm: isize = (((el_x_len - 1) as f64) / 3.0).floor() as isize - 1;
                if mm < 0 {
                    mm = 0;
                }
                let mut m = mm as usize;
                let mut x_avg = el_x[l][m];
                let mut y_avg = el_y[l][m];
                while m < el_x_len {
                    let xm = (el_x[l][m] - xstart) / size;
                    let ym = (el_y[l][m] - ystart) / size;
                    if m < el_x_len - 3
                        && ym == ym.floor()
                        && (xm - xm.floor()).abs() > 0.5
                        && ym.floor() != ((el_y[l][0] - ystart) / size).floor()
                        && xm.floor() != ((el_x[l][0] - xstart) / size).floor()
                    {
                        x_avg = xm.floor() * size + xstart;
                        y_avg = el_y[l][m].floor();
                        m += el_x_len;
                    }
                    m += 1;
                }
                let foo_x = ((x_avg - xstart) / size).floor() as u64;
                let foo_y = ((y_avg - ystart) / size).floor() as u64;

                let h_center = *xyz.get(&(foo_x, foo_y)).unwrap_or(&0.0);

                let mut hit = 0;

                let xtest = foo_x as f64 * size + xstart;
                let ytest = foo_y as f64 * size + ystart;

                let mut x0 = f64::NAN;
                let mut y0 = f64::NAN;
                for n in 0..el_x[l].len() {
                    let x1 = el_x[l][n];
                    let y1 = el_y[l][n];
                    if n > 0
                        && ((y0 <= ytest && ytest < y1) || (y1 <= ytest && ytest < y0))
                        && (xtest < (x1 - x0) * (ytest - y0) / (y1 - y0) + x0)
                    {
                        hit += 1;
                    }
                    x0 = x1;
                    y0 = y1;
                }
                depression = 1;
                if (h_center < h && hit % 2 == 1) || (h_center > h && hit % 2 != 1) {
                    depression = -1;
                    write!(&mut depr_fp, "{},{}", el_x[l][0], el_y[l][0])
                        .expect("Unable to write file");
                    for k in 1..el_x[l].len() {
                        write!(&mut depr_fp, "|{},{}", el_x[l][k], el_y[l][k])
                            .expect("Unable to write file");
                    }
                    writeln!(&mut depr_fp).expect("Unable to write file");
                }
                if !skip {
                    // Check if knoll is distinct enough
                    let mut steepcounter = 0;
                    let mut minele = f64::MAX;
                    let mut maxele = f64::MIN;
                    for k in 0..(el_x_len - 1) {
                        let xx = ((el_x[l][k] - xstart) / size + 0.5).floor() as usize;
                        let yy = ((el_y[l][k] - ystart) / size + 0.5).floor() as usize;
                        let ss = steepness[xx][yy];
                        if minele > h - 0.5 * ss {
                            minele = h - 0.5 * ss;
                        }
                        if maxele < h + 0.5 * ss {
                            maxele = h + 0.5 * ss;
                        }
                        if ss > 1.0 {
                            steepcounter += 1;
                        }
                    }

                    if (steepcounter as f64) < 0.4 * (el_x_len as f64 - 1.0)
                        && el_x_len < 41
                        && depression as f64 * h_center - 1.9 < minele
                    {
                        if maxele - 0.45 * scalefactor * inidotknolls < minele {
                            skip = true;
                        }
                        if el_x_len < 33 && maxele - 0.75 * scalefactor * inidotknolls < minele {
                            skip = true;
                        }
                        if el_x_len < 19 && maxele - 0.9 * scalefactor * inidotknolls < minele {
                            skip = true;
                        }
                    }
                    if (steepcounter as f64) < inidotknolls * (el_x_len - 1) as f64 && el_x_len < 15
                    {
                        skip = true;
                    }
                }
            }
            if el_x_len < 5 {
                skip = true;
            }
            if !skip && el_x_len < 15 {
                // dot knoll
                let mut x_avg = 0.0;
                let mut y_avg = 0.0;
                for k in 0..(el_x_len - 1) {
                    x_avg += el_x[l][k];
                    y_avg += el_y[l][k];
                }
                x_avg /= (el_x_len - 1) as f64;
                y_avg /= (el_x_len - 1) as f64;
                write!(&mut dotknoll_fp, "{} {} {}\r\n", depression, x_avg, y_avg)
                    .expect("Unable to write to file");
                skip = true;
            }

            if !skip {
                // not skipped, lets save first coordinate pair for later form line knoll PIP analysis
                write!(&mut knollhead_fp, "{} {}\r\n", el_x[l][0], el_y[l][0])
                    .expect("Unable to write to file");
                // adaptive generalization
                if el_x_len > 101 {
                    let mut newx: Vec<f64> = vec![];
                    let mut newy: Vec<f64> = vec![];
                    let mut xpre = el_x[l][0];
                    let mut ypre = el_y[l][0];

                    newx.push(el_x[l][0]);
                    newy.push(el_y[l][0]);

                    for k in 1..(el_x_len - 1) {
                        let xx = ((el_x[l][k] - xstart) / size + 0.5).floor() as usize;
                        let yy = ((el_y[l][k] - ystart) / size + 0.5).floor() as usize;
                        let ss = steepness[xx][yy];
                        if ss.is_nan() || ss < 0.5 {
                            if ((xpre - el_x[l][k]).powi(2) + (ypre - el_y[l][k]).powi(2)).sqrt()
                                >= 4.0
                            {
                                newx.push(el_x[l][k]);
                                newy.push(el_y[l][k]);
                                xpre = el_x[l][k];
                                ypre = el_y[l][k];
                            }
                        } else {
                            newx.push(el_x[l][k]);
                            newy.push(el_y[l][k]);
                            xpre = el_x[l][k];
                            ypre = el_y[l][k];
                        }
                    }
                    newx.push(el_x[l][el_x_len - 1]);
                    newy.push(el_y[l][el_x_len - 1]);

                    el_x[l].clear();
                    el_x[l].append(&mut newx);
                    el_y[l].clear();
                    el_y[l].append(&mut newy);
                    el_x_len = el_x[l].len();
                }
                // Smoothing
                let mut dx: Vec<f64> = vec![f64::NAN; el_x_len];
                let mut dy: Vec<f64> = vec![f64::NAN; el_x_len];

                for k in 2..(el_x_len - 3) {
                    dx[k] = (el_x[l][k - 2]
                        + el_x[l][k - 1]
                        + el_x[l][k]
                        + el_x[l][k + 1]
                        + el_x[l][k + 2]
                        + el_x[l][k + 3])
                        / 6.0;
                    dy[k] = (el_y[l][k - 2]
                        + el_y[l][k - 1]
                        + el_y[l][k]
                        + el_y[l][k + 1]
                        + el_y[l][k + 2]
                        + el_y[l][k + 3])
                        / 6.0;
                }

                let mut xa: Vec<f64> = vec![f64::NAN; el_x_len];
                let mut ya: Vec<f64> = vec![f64::NAN; el_x_len];
                for k in 1..(el_x_len - 1) {
                    xa[k] = (el_x[l][k - 1] + el_x[l][k] / (0.01 + smoothing) + el_x[l][k + 1])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                    ya[k] = (el_y[l][k - 1] + el_y[l][k] / (0.01 + smoothing) + el_y[l][k + 1])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                }

                if el_x[l].first() == el_x[l].last() && el_y[l].first() == el_y[l].last() {
                    let vx = (el_x[l][1] + el_x[l][0] / (0.01 + smoothing) + el_x[l][el_x_len - 2])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                    let vy = (el_y[l][1] + el_y[l][0] / (0.01 + smoothing) + el_y[l][el_x_len - 2])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                    xa[0] = vx;
                    ya[0] = vy;
                    xa[el_x_len - 1] = vx;
                    ya[el_x_len - 1] = vy;
                } else {
                    xa[0] = el_x[l][0];
                    ya[0] = el_y[l][0];
                    xa[el_x_len - 1] = el_x[l][el_x_len - 1];
                    ya[el_x_len - 1] = el_y[l][el_x_len - 1];
                }
                for k in 1..(el_x_len - 1) {
                    el_x[l][k] = (xa[k - 1] + xa[k] / (0.01 + smoothing) + xa[k + 1])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                    el_y[l][k] = (ya[k - 1] + ya[k] / (0.01 + smoothing) + ya[k + 1])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                }
                if xa.first() == xa.last() && ya.first() == ya.last() {
                    let vx = (xa[1] + xa[0] / (0.01 + smoothing) + xa[el_x_len - 2])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                    let vy = (ya[1] + ya[0] / (0.01 + smoothing) + ya[el_x_len - 2])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                    el_x[l][0] = vx;
                    el_y[l][0] = vy;
                    el_x[l][el_x_len - 1] = vx;
                    el_y[l][el_x_len - 1] = vy;
                } else {
                    el_x[l][0] = xa[0];
                    el_y[l][0] = ya[0];
                    el_x[l][el_x_len - 1] = xa[el_x_len - 1];
                    el_y[l][el_x_len - 1] = ya[el_x_len - 1];
                }

                for k in 1..(el_x_len - 1) {
                    xa[k] = (el_x[l][k - 1] + el_x[l][k] / (0.01 + smoothing) + el_x[l][k + 1])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                    ya[k] = (el_y[l][k - 1] + el_y[l][k] / (0.01 + smoothing) + el_y[l][k + 1])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                }

                if el_x[l].first() == el_x[l].last() && el_y[l].first() == el_y[l].last() {
                    let vx = (el_x[l][1] + el_x[l][0] / (0.01 + smoothing) + el_x[l][el_x_len - 2])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                    let vy = (el_y[l][1] + el_y[l][0] / (0.01 + smoothing) + el_y[l][el_x_len - 2])
                        / (2.0 + 1.0 / (0.01 + smoothing));
                    xa[0] = vx;
                    ya[0] = vy;
                    xa[el_x_len - 1] = vx;
                    ya[el_x_len - 1] = vy;
                } else {
                    xa[0] = el_x[l][0];
                    ya[0] = el_y[l][0];
                    xa[el_x_len - 1] = el_x[l][el_x_len - 1];
                    ya[el_x_len - 1] = el_y[l][el_x_len - 1];
                }
                for k in 0..el_x_len {
                    el_x[l][k] = xa[k];
                    el_y[l][k] = ya[k];
                }

                let mut dx2: Vec<f64> = vec![f64::NAN; el_x_len];
                let mut dy2: Vec<f64> = vec![f64::NAN; el_x_len];
                for k in 2..(el_x_len - 3) {
                    dx2[k] = (el_x[l][k - 2]
                        + el_x[l][k - 1]
                        + el_x[l][k]
                        + el_x[l][k + 1]
                        + el_x[l][k + 2]
                        + el_x[l][k + 3])
                        / 6.0;
                    dy2[k] = (el_y[l][k - 2]
                        + el_y[l][k - 1]
                        + el_y[l][k]
                        + el_y[l][k + 1]
                        + el_y[l][k + 2]
                        + el_y[l][k + 3])
                        / 6.0;
                }
                for k in 3..(el_x_len - 3) {
                    let vx = el_x[l][k] + (dx[k] - dx2[k]) * curviness;
                    let vy = el_y[l][k] + (dy[k] - dy2[k]) * curviness;
                    el_x[l][k] = vx;
                    el_y[l][k] = vy;
                }

                let mut layer = String::from("contour");
                if depression == -1 {
                    layer = String::from("depression");
                }
                if indexcontours != 0.0
                    && (((h / interval + 0.5).floor() * interval) / indexcontours).floor()
                        - ((h / interval + 0.5).floor() * interval) / indexcontours
                        == 0.0
                {
                    layer.push_str("_index");
                }
                if formline > 0.0
                    && (((h / interval + 0.5).floor() * interval) / (2.0 * interval)).floor()
                        - ((h / interval + 0.5).floor() * interval) / (2.0 * interval)
                        != 0.0
                {
                    layer.push_str("_intermed");
                }
                out.push_str(
                    format!(
                        "POLYLINE\r\n 66\r\n1\r\n  8\r\n{}\r\n 38\r\n{}\r\n  0\r\n",
                        layer, h
                    )
                    .as_str(),
                );
                for k in 0..el_x_len {
                    out.push_str(
                        format!(
                            "VERTEX\r\n  8\r\n{}\r\n 10\r\n{}\r\n 20\r\n{}\r\n 30\r\n{}\r\n  0\r\n",
                            layer, el_x[l][k], el_y[l][k], h
                        )
                        .as_str(),
                    );
                }
                out.push_str("SEQEND\r\n  0\r\n");
            } // -- if not dotkoll
        }
    }
    out.push_str("ENDSEC\r\n  0\r\nEOF\r\n");
    let output = tile.dir_path.join("contours.dxf");
    let fp = File::create(output).expect("Unable to create file");
    let mut fp = BufWriter::new(fp);
    fp.write_all(out.as_bytes()).expect("Unable to write file");
    println!("Done");
    Ok(())
}
