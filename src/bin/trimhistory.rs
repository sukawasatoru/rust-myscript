extern crate chrono;
extern crate env_logger;
extern crate getopts;
#[macro_use]
extern crate log;

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

fn main() {
    env_logger::init();

    info!("Hello");

    let file_path = get_history_path();
    debug!("input {:?}", file_path);

    let history_file = File::open(&file_path).unwrap();
    let mut lines = BufReader::new(&history_file).lines();

    let mut trimed: Vec<String> = Vec::new();
    let mut trim_count = 0;
    loop {
        match lines.next() {
            Some(line) => {
                debug!("result: {:?}", line);
                let a = line.unwrap();
                if let Some(index) = trimed.iter().position(|entity| &a == entity) {
                    debug!("contains: {}", index);
                    trimed.remove(index);
                    trim_count += 1;
                }
                trimed.push(a);
            }
            None => break,
        }
    }

    debug!("trim_count: {}, len: {}", trim_count, trimed.len());

    std::fs::copy(&file_path, generate_backup_path(&file_path)).unwrap();
    let out_file = File::create(&file_path).unwrap();
    let mut writer = BufWriter::new(out_file);
    for entity in trimed.iter() {
        writeln!(&mut writer, "{}", entity).unwrap();
    }
    writer.flush().unwrap();

    info!("Bye");
}

fn get_history_path() -> PathBuf {
    let options = getopts::Options::new();
    let args = std::env::args().collect::<Vec<String>>();
    let matches = options.parse(&args[1..]).unwrap();
    std::path::Path::new(matches.free.get(0).unwrap()).to_path_buf()
}

fn generate_backup_path(path: &Path) -> PathBuf {
    let mut file_name = path.file_name().unwrap().to_os_string();
    let now = chrono::Local::now();
    file_name.push(now.format(".%Y%m%d-%H%M%S").to_string());
    path.with_file_name(&file_name)
}
