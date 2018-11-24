use std::env;
use std::io::{Read, Write};
use std::fs::File;
use std::path::Path;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let graphql_path = Path::new(&manifest_dir).join("graphql_release.txt");
    let mut file = File::open(&graphql_path).unwrap();
    let mut graphql_release = String::new();
    file.read_to_string(&mut graphql_release).unwrap();

    let out_dir = env::var("OUT_DIR").unwrap();
    let checkghossversion_token_path = Path::new(&out_dir).join("checkghossversion_token.rs");
    let mut checkghossversion_file = File::create(&checkghossversion_token_path).unwrap();
    // TODO: r#.
    checkghossversion_file.write_all(&format!("
pub fn get_checkghossversion_string() -> &'static str {{
    \"{}\"
}}", &graphql_release).into_bytes()).unwrap();
}

