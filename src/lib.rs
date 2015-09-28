#![feature(libc)]

extern crate image;
extern crate rand;
extern crate rustc_serialize;
extern crate cbor;
extern crate libc;

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::thread;
use std::sync::Arc;
use std::sync::mpsc;
use std::io::{Write, BufReader};
use std::error::Error;

use image::GenericImage;
use cbor::{Decoder, Encoder};
use std::ffi::CStr;

const W_NUMB: u32 = 10;
const H_NUMB: u32 = 10;
const MY_IMAGES_DIR: &'static str = "images_db";
const RESULTS_DIR: &'static str = "results";

#[derive(Hash, Eq, PartialEq, Debug, Copy, Clone, RustcEncodable, RustcDecodable)]
struct MyColor {
    r: u8,
    g: u8,
    b: u8,
}

impl MyColor {
    fn new(r: u8, g: u8, b: u8) -> MyColor {
        MyColor {
            r: r,
            g: g,
            b: b,
        }
    }
    fn distance(&self, other: &MyColor) -> f32 {
        let dist: f32 =
        ((self.r as f32 - other.r as f32).powi(2) +
        (self.g as f32 - other.g as f32).powi(2) +
        (self.b as f32 - other.b as f32).powi(2)).sqrt();
        dist
    }
}

fn average_color(file: &Path) -> Option<MyColor> {
    let im = match image::open(file) {
        Ok(file) => {file},
        Err(e) => {println!("cant opent image, {}", e);return None}
    };

    let size = im.dimensions();
    let wh = size.0 * size.1;

    let mut avg_color = (0f32, 0f32, 0f32);

    for x in 0..size.0 {
        for y in 0..size.1 {
            let pixel = im.get_pixel(x, y);
            avg_color = (
                (avg_color.0 + pixel.data[0] as f32/wh as f32),
                (avg_color.1 + pixel.data[1] as f32/wh as f32),
                (avg_color.2 + pixel.data[2] as f32/wh as f32)
           );
        }
    }
    let color = MyColor::new(avg_color.0 as u8, avg_color.1 as u8, avg_color.2 as u8);
    Some(color)
}

fn process_image_single(path: &Path, db: &HashMap<MyColor, Vec<PathBuf>>) -> Result<i32, Box<Error>> {
    let parent_image = try!(image::open(path));
    let parent_size = parent_image.dimensions();

    let (wi, hi) = (parent_size.0/W_NUMB, parent_size.1/H_NUMB);
    let child_size = (wi*W_NUMB, hi*H_NUMB);

    let mut imgbuf = image::ImageBuffer::new(child_size.0, child_size.1);
    let mut tmp_piece_color: MyColor;
    let mut rand_gen = rand::thread_rng();

    for i in 0..hi {
        for j in 0..wi {
            let pixel = parent_image.get_pixel(j*W_NUMB,i*H_NUMB);
            tmp_piece_color = MyColor::new(
                pixel.data[0],
                pixel.data[1],
                pixel.data[2],
                );
            let nearest_img: MyColor = nearest_color(&tmp_piece_color, db);
            {
                let image_from = db.get(&nearest_img).unwrap();
                let index: usize = rand::sample(&mut rand_gen, 0..image_from.len(), 1)[0];
                imgbuf.copy_from(&image::open(&image_from[index]).unwrap(), j*W_NUMB, i*H_NUMB);
            }
        }
    }

    let ref mut fout = File::create(&Path::new(RESULTS_DIR).join(path.file_stem().unwrap()).with_extension("png")).unwrap();
    let _ = image::ImageRgba8(imgbuf).save(fout, image::PNG);
    Ok(0)
}

fn process_image(path: &Path, db: HashMap<MyColor, Vec<PathBuf>>) -> Result<i32, Box<Error>> {
    let parent_image = Arc::new(try!(image::open(path)));
    let parent_size = parent_image.dimensions();

    let (wi, hi) = (parent_size.0/W_NUMB, parent_size.1/H_NUMB);
    let child_size = (wi*W_NUMB, hi*H_NUMB);

    let (tx, rx) = mpsc::channel();
    let shared_db = Arc::new(db);
    for t in 0..4 {
        let (shared_db, tx, parent_image) = (shared_db.clone(), tx.clone(), parent_image.clone());
        thread::spawn(move || {
            let mut tmp_piece_color: MyColor;
            let mut rand_gen = rand::thread_rng();
            let mut imgbuf = image::ImageBuffer::new(child_size.0/2, child_size.1/2);

            let (from_x, to_x, from_y, to_y) = match t {
                0 => (0, wi/2, 0, hi/2),
                1 => (wi/2, wi, 0, hi/2),
                2 => (0, wi/2, hi/2, hi),
                3 => (wi/2, wi, hi/2, hi),
                _ => panic!("Incorrect index"),
            };

            for i in from_y..to_y {
                for j in from_x..to_x {
                    let pixel = parent_image.get_pixel(j*W_NUMB,i*H_NUMB);
                    tmp_piece_color = MyColor::new(
                        pixel.data[0],
                        pixel.data[1],
                        pixel.data[2],
                        );
                    let nearest_img: MyColor = nearest_color(&tmp_piece_color, &shared_db);
                    let image_from = shared_db.get(&nearest_img).unwrap();
                    let index: usize = rand::sample(&mut rand_gen, 0..image_from.len(), 1)[0];
                    imgbuf.copy_from(&image::open(&image_from[index]).unwrap(), (j - from_x)*W_NUMB, (i - from_y)*H_NUMB);
                }
            }

            tx.send((imgbuf, t)).unwrap();
        });
    }
    let mut imgbuf = image::ImageBuffer::new(child_size.0, child_size.1);

    for _ in 0..4 {
        let (image_part, index) = rx.recv().unwrap();
        match index {
            0 => {imgbuf.copy_from(&image_part, 0, 0)},
            1 => {imgbuf.copy_from(&image_part, child_size.0/2, 0)},
            2 => {imgbuf.copy_from(&image_part, 0, child_size.1/2)},
            3 => {imgbuf.copy_from(&image_part, child_size.0/2, child_size.1/2)},
            _ => {panic!("Incorrect index")},
        };
    }

    let ref mut fout = File::create(&Path::new(RESULTS_DIR).join(path.file_stem().unwrap()).with_extension("png")).unwrap();
    let _ = image::ImageRgba8(imgbuf).save(fout, image::PNG);
    Ok(0)
}

fn create_db() ->  Option<HashMap<MyColor, Vec<PathBuf>>> {
    let mut db: HashMap<MyColor, Vec<PathBuf>> = HashMap::new();

    for entry in fs::read_dir(MY_IMAGES_DIR).unwrap() {
        let entry = entry.unwrap();
        match average_color(&entry.path()) {
            Some(avg_color) => {
                        if db.contains_key(&avg_color) {
                            let mut key = db.get_mut(&avg_color).unwrap();
                            //println!("picture with the same color");
                            key.push(entry.path());
                        }
                        else {
                            db.insert(avg_color, vec!(entry.path()));
                        }
                    },
            None => {println!("can not calculate average color")},
        };
    }

    match db.capacity() {
        0 => {println!("There are no images in db. You need to scan folder with images"); panic!();},
        _ => {write_db_to_file(&db);Some(db)}
    }
}

fn collect_images(directory: &Path) {
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        if fs::metadata(entry.path()).unwrap().is_dir() {
            collect_images(&entry.path());
        }
        else {
            if entry.path().extension().unwrap() == "jpg" || entry.path().extension().unwrap() == "png" {
                //println!("{:?}", entry.path());
                match fs::metadata(&Path::new("images_db").join(entry.path().file_stem().unwrap()).with_extension("jpg")) {
                        Ok(_) => {},//println!("This name already used")},
                        Err(_) => {
                            match image::open(entry.path()) {
                                Ok(img) => {
                                            let ref mut fout = File::create(&Path::new("images_db")
                                            .join(entry.path().file_stem().unwrap())
                                            .with_extension("jpg")).unwrap();

                                            let imgbuf = img.resize_exact(W_NUMB, H_NUMB, image::FilterType::Lanczos3);
                                            let _ = imgbuf.save(fout, image::JPEG);
                                            println!("{}, moved to db folder", entry.path().to_str().unwrap());
                                            },
                            Err(e) => {println!("{}, fail open image: {}", entry.path().to_str().unwrap(), e)},
                            };
                        }
                };
            }
        }
    }
}

fn nearest_color(color: &MyColor, db: &HashMap<MyColor, Vec<PathBuf>>) -> MyColor {
    let mut nearest: &MyColor = color;
    let mut distance: f32;
    let mut min_distance: f32 = 10000_f32;
    {
        for key in db.keys() {
            distance = color.distance(&key);
            if min_distance > distance {
                nearest = key;
                min_distance = distance;
            }
        }
    }
    *nearest
}

fn write_db_to_file(db: &HashMap<MyColor, Vec<PathBuf>>) {
    let mut encoder = Encoder::from_memory();
    encoder.encode(db).unwrap();
    let mut db_out = File::create(&Path::new("db.bin")).unwrap();
    db_out.write(&encoder.as_bytes()).unwrap();
}

#[test]
fn main_test(){
    let path_to_proc_image = "/home/nikon/Изображения/YHaG4NYK5DM.jpg".to_string();
    let scan = false;
    let path_to_scan_folder = "/home/nikon/Изображения/вычмат/".to_string();
    let single = false;

    match fs::metadata(MY_IMAGES_DIR) {
        Ok(_) => {},
        Err(_) => {fs::create_dir(MY_IMAGES_DIR).unwrap();println!("Folder created: {}", MY_IMAGES_DIR);}
    };

    match fs::metadata(RESULTS_DIR) {
        Ok(_) => {},
        Err(_) => {fs::create_dir(RESULTS_DIR).unwrap();println!("Folder created: {}", RESULTS_DIR);}
    };

    if scan {
        let img_folder = Path::new(&path_to_scan_folder);
        collect_images(&img_folder);

        let _db: HashMap<MyColor, Vec<PathBuf>> = create_db().unwrap();
        return;
    }

    let file_path = Path::new(&path_to_proc_image);
    match fs::metadata(&file_path) {
        Ok(_) => {},
        Err(_) => {println!("File that you gave does not exist"); return},
    };

    let db: HashMap<MyColor, Vec<PathBuf>>;

    match File::open(&Path::new("db.bin")) {
        Ok(file) => {
                let reader = BufReader::new(&file);
                let mut decoder = Decoder::from_reader(reader);
                db = decoder.decode().collect::<Result<_, _>>().unwrap();
            },
        Err(_) => {db = create_db().unwrap()}
    }

    let result;

    if single {
        result = match process_image_single(&file_path, &db) {
            Ok(res) => res,
            Err(_) => {-1},
        };
    }
    else {
        result = match process_image(&file_path, db) {
            Ok(res) => res,
            Err(_) => {-1},
        };
    }

    println!("Allright. New image was successfully created.");
}

#[no_mangle]
pub extern fn main_work(path_to_proc_image: *const libc::c_char, scan: bool, path_to_scan_folder: *const libc::c_char, single: bool) -> i32 {
    let path_to_proc_image = unsafe { CStr::from_ptr(path_to_proc_image).to_string_lossy().into_owned() };
    let path_to_scan_folder = unsafe { CStr::from_ptr(path_to_scan_folder).to_string_lossy().into_owned() };

    match fs::metadata(MY_IMAGES_DIR) {
        Ok(_) => {},
        Err(_) => {fs::create_dir(MY_IMAGES_DIR).unwrap();println!("Folder created: {}", MY_IMAGES_DIR);}
    };

    match fs::metadata(RESULTS_DIR) {
        Ok(_) => {},
        Err(_) => {fs::create_dir(RESULTS_DIR).unwrap();println!("Folder created: {}", RESULTS_DIR);}
    };

    if scan {
        let img_folder = Path::new(&path_to_scan_folder);
        collect_images(&img_folder);

        let _db: HashMap<MyColor, Vec<PathBuf>> = create_db().unwrap();
        return 0;
    }

    let file_path = Path::new(&path_to_proc_image);
    match fs::metadata(&file_path) {
        Ok(_) => {},
        Err(_) => {println!("File that you gave does not exist"); return -1},
    };

    let db: HashMap<MyColor, Vec<PathBuf>>;

    match File::open(&Path::new("db.bin")) {
        Ok(file) => {
                let reader = BufReader::new(&file);
                let mut decoder = Decoder::from_reader(reader);
                db = decoder.decode().collect::<Result<_, _>>().unwrap();
            },
        Err(_) => {db = create_db().unwrap()}
    }

    let result;

    if single {
        result = match process_image_single(&file_path, &db) {
            Ok(res) => res,
            Err(_) => {return -1},
        };
    }
    else {
        result = match process_image(&file_path, db) {
            Ok(res) => res,
            Err(_) => {return -1},
        };
    }

    println!("Allright. New image was successfully created.");

    result
}
