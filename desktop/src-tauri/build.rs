use std::{env, fs, fs::File, path::PathBuf};

fn main() {
    // Tauri's context macro expects a default icon even while bundling is
    // disabled. Keep this tiny development placeholder until the product
    // icon is designed.
    let icon_path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("icons")
        .join("icon.png");
    fs::create_dir_all(icon_path.parent().unwrap()).unwrap();
    let file = File::create(icon_path).unwrap();
    let mut encoder = png::Encoder::new(file, 1, 1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&[24, 24, 31, 255])
        .unwrap();
    tauri_build::build()
}
