use std::env;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use failure::format_err;

type Result<T> = std::result::Result<T, failure::Error>;

fn main() -> Result<()> {
    checkghossversion()
}

fn checkghossversion() -> Result<()> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let asset_path = Path::new(&manifest_dir).join("asset");

    let mut methods = Vec::new();
    let mut file_string = String::new();
    for file_name in ["fragment_release", "fragment_tag"].iter() {
        let file_path = asset_path.join(format!("{}.graphql", file_name));
        let mut file = match File::open(&file_path) {
            Ok(ok) => ok,
            Err(e) => Err(format_err!("Failed to open file: {:?}, {:?}", file_path, e))?,
        };
        file_string.clear();
        file.read_to_string(&mut file_string)?;
        methods.push(
            format!(
                r##"pub fn get_{}() -> &'static str {{
    r#"{}"#
}}
"##,
                file_name, &file_string
            )
            .into_bytes(),
        );
    }

    let out_dir = env::var("OUT_DIR")?;
    let mut checkghossversion_file =
        File::create(Path::new(&out_dir).join("checkghossversion_token.rs"))?;

    for method_str in &methods {
        checkghossversion_file.write_all(method_str)?;
    }
    Ok(())
}
