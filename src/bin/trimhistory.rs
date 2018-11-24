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

    if let Some(backup_path) = opt.backup_path {
        std::fs::copy(&file_path, backup_path).unwrap();
    }
    let out_file = File::create(&file_path).unwrap();
    let mut writer = BufWriter::new(out_file);
    for entity in trimed.iter() {
        writeln!(&mut writer, "{}", entity).unwrap();
    }
    writer.flush().unwrap();

    info!("Bye");
}
