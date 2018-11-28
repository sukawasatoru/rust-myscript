extern crate directories;
extern crate dotenv;
extern crate env_logger;
#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;
extern crate structopt;
extern crate toml;

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

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

#[derive(Debug, Deserialize, Serialize)]
struct Entry {
    command: String,
    count: i32,
}

impl Entry {
    fn new(command: &str) -> Entry {
        Entry {
            command: command.to_owned(),
            count: 1,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct Statistics {
    entries: Vec<Entry>,
}

impl Statistics {
    fn new() -> Statistics {
        Statistics {
            entries: Vec::new(),
        }
    }

    fn find_command(&self, command: &str) -> Result<usize, ()> {
        for i in 0..self.entries.len() {
            if self.entries[i].command == command {
                return Ok(i)
            }
        }
        Err(())
    }
}

fn main() {
    dotenv::dotenv().ok();
    env_logger::init();

    info!("Hello");

    let opt: Opt = Opt::from_args();
    debug!("config: {:?}", opt);

    let file_path = opt.history_path;
    debug!("input {:?}", file_path);
    let project_dirs = directories::ProjectDirs::from(
        "jp", "tinyport", "trimhistory").unwrap();
    let statistics_path = project_dirs.data_dir().join("statistics.toml");

    let mut statistics = if statistics_path.exists() {
        load_statistics(&statistics_path).unwrap()
    } else {
        Statistics::new()
    };

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
                        increment_command_count(&mut statistics, trimmed_line);
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

    store_statistics(&statistics_path, &statistics).unwrap();

    info!("Bye");
}

fn load_statistics(path: &Path) -> Result<Statistics, ()> {
    let statistics_file = match File::open(&path) {
        Ok(file) => file,
        Err(e) => panic!(e),
    };
    let mut buf = BufReader::new(statistics_file);
    let mut statistics_data = Vec::new();
    if let Err(e) = buf.read_to_end(&mut statistics_data) {
        panic!(e);
    }
    match toml::from_slice::<Statistics>(&statistics_data) {
        Ok(ok) => Ok(ok),
        Err(e) => panic!(e),
    }
}

fn store_statistics(path: &Path, statistics: &Statistics) -> Result<(), ()> {
    use std::fs;
    let data_dir = path.parent().unwrap();
    if !data_dir.exists() {
        if let Err(e) = fs::create_dir_all(data_dir) {
            panic!(e);
        }
    }
    match File::create(&path) {
        Ok(file) => {
            let mut writer = BufWriter::new(file);
            let statistics_data = match toml::to_vec(&statistics) {
                Ok(data) => data,
                Err(e) => panic!(e),
            };
            if let Err(e) = writer.write_all(&statistics_data) {
                panic!(e);
            }
            Ok(())
        }
        Err(e) => panic!(e),
    }
}

fn increment_command_count(statistics: &mut Statistics, command: &str) {
    match statistics.find_command(command) {
        Ok(index) => statistics.entries[index].count += 1,
        Err(_) => statistics.entries.push(Entry::new(command)),
    }
}
