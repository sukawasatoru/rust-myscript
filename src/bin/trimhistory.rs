extern crate dotenv;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate structopt;

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;

use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "trimhistory")]
struct Opt {
    #[structopt(
    short = "b", long = "backup", help = "Backup a FILE to specified path",
    parse(from_os_str)
    )]
    backup_path: Option<PathBuf>,

    #[structopt(name = "FILE", help = "history file", parse(from_os_str))]
    history_path: PathBuf,
}

fn main() {
    dotenv::dotenv().ok();
    env_logger::init();

    info!("Hello");

    let opt: Opt = Opt::from_args();
    debug!("config: {:?}", opt);

    let file_path = opt.history_path;
    debug!("input {:?}", file_path);

    let history_file = File::open(&file_path).unwrap();
    let mut buffer = BufReader::new(&history_file);
    let mut line = String::new();
    let mut trimmed = Vec::new();
    let mut trim_count = 0;
    loop {
        match buffer.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line.ends_with("\n") {
                    line.pop();
                    if line.ends_with("\r") {
                        line.pop();
                    }
                }
                debug!("result: {:?}", line);
                {
                    let trimmed_line = line.trim();
                    if let Some(index) = trimmed.iter().position(|entity| trimmed_line == entity) {
                        debug!("contains: {}", index);
                        trimmed.remove(index);
                        trim_count += 1;
                    }
                    trimmed.push(trimmed_line.to_owned());
                }
                line.clear();
            }
            Err(e) => panic!(e),
        }
    }

    info!("trim_count: {}, len: {}", trim_count, trimmed.len());

    if let Some(backup_path) = opt.backup_path {
        std::fs::copy(&file_path, backup_path).unwrap();
    }
    let out_file = File::create(&file_path).unwrap();
    let mut writer = BufWriter::new(out_file);
    for entity in trimmed.iter() {
        writeln!(&mut writer, "{}", entity).unwrap();
    }
    writer.flush().unwrap();

    info!("Bye");
}
