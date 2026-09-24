use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::PathBuf;

fn main() {
    tauri_build::build();

    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let assets_dir = manifest_dir.join("resources/weapons");
    println!("cargo:rerun-if-changed={}", assets_dir.display());

    let mut assets = String::from(
        "pub(crate) fn weapon_image(row_id: &str) -> Option<&'static [u8]> {\n    match row_id {\n",
    );
    let mut files = fs::read_dir(&assets_dir)
        .expect("read bundled weapon images")
        .map(|entry| entry.expect("read weapon asset entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "png"))
        .collect::<Vec<_>>();
    files.sort();

    for path in files {
        let Some(row_id) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        if !row_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            continue;
        }
        let relative_path = path
            .strip_prefix(&manifest_dir)
            .expect("weapon image is inside the crate")
            .to_string_lossy()
            .replace('\\', "/");
        writeln!(
            assets,
            "        {row_id:?} => Some(include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{relative_path}\"))),"
        )
        .expect("write weapon asset entry");
    }
    assets.push_str("        _ => None,\n    }\n}\n");

    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("weapon_assets.rs");
    fs::write(output, assets).expect("write bundled weapon asset lookup");
}
