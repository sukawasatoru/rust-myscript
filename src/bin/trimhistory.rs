use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
};

use log::{debug, info};
use serde_derive::{Deserialize, Serialize};
use structopt::StructOpt;

use rust_myscript::myscript::prelude::*;

#[derive(StructOpt, Debug)]
#[structopt(name = "trimhistory")]
struct Opt {
    #[structopt(subcommand)]
    cmd: Command,
}

#[derive(StructOpt, Debug)]
enum Command {
    #[structopt(name = "trim")]
    Trim {
        #[structopt(
            short = "b",
            long = "backup",
            help = "Backup a FILE to specified path",
            parse(from_os_str)
        )]
        backup_path: Option<PathBuf>,

        #[structopt(name = "FILE", help = "history file", parse(from_os_str))]
        history_path: PathBuf,
    },
    #[structopt(name = "show")]
    Show {
        #[structopt(
            name = "NUM",
            short = "n",
            long = "lines",
            help = "prints the first NUM lines"
        )]
        num: Option<i32>,
    },
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

    fn find_command(&self, command: &str) -> Option<usize> {
        for i in 0..self.entries.len() {
            if self.entries[i].command == command {
                return Some(i);
            }
        }
        None
    }
}

fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    env_logger::init();

    let opt: Opt = Opt::from_args();
    debug!("config: {:?}", opt);

    match opt.cmd {
        Command::Trim {
            backup_path,
            history_path,
        } => trim(history_path, backup_path),
        Command::Show { num } => show(num),
    }
}

fn trim(history_path: PathBuf, backup_path: Option<PathBuf>) -> Fallible<()> {
    debug!("input {:?}", history_path);
    let project_dirs: directories::ProjectDirs =
        directories::ProjectDirs::from("jp", "tinyport", "trimhistory").ok_or_err()?;
    let statistics_path = project_dirs.data_dir().join("statistics.toml");

    let mut statistics = if statistics_path.exists() {
        load_statistics(&statistics_path)?
    } else {
        Statistics::new()
    };

    let history_file = File::open(&history_path)?;
    let mut buffer = BufReader::new(&history_file);
    let mut line = String::new();
    let mut trimmed = Vec::new();
    let mut trim_count = 0;
    loop {
        match buffer.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                debug!("result: {:?}", line);
                let trimmed_line = line.trim();
                if let Some(index) = trimmed.iter().position(|entity| trimmed_line == entity) {
                    debug!("contains: {}", index);
                    trimmed.remove(index);
                    trim_count += 1;
                    increment_command_count(&mut statistics, trimmed_line);
                }
                trimmed.push(trimmed_line.to_owned());
                line.clear();
            }
            Err(e) => return Err(e.into()),
        }
    }

    info!("trim_count: {}, len: {}", trim_count, trimmed.len());

    if let Some(backup_path) = backup_path {
        std::fs::copy(&history_path, backup_path)?;
    }
    let out_file = File::create(&history_path)?;
    let mut writer = BufWriter::new(out_file);
    for entity in trimmed.iter() {
        writeln!(&mut writer, "{}", entity)?;
    }
    writer.flush()?;

    store_statistics(&statistics_path, &statistics)?;

    Ok(())
}

fn show(num: Option<i32>) -> Fallible<()> {
    let project_dirs: directories::ProjectDirs =
        directories::ProjectDirs::from("jp", "tinyport", "trimhistory").ok_or_err()?;
    let statistics_path = project_dirs.data_dir().join("statistics.toml");

    let mut statistics: Statistics = load_statistics(&statistics_path)?;

    statistics.entries.sort_by(|lh, rh| rh.count.cmp(&lh.count));
    let len = if let Some(num) = num {
        if statistics.entries.len() < num as usize {
            statistics.entries.len()
        } else {
            num as usize
        }
    } else {
        statistics.entries.len()
    };
    for entry in &statistics.entries[..len] {
        println!("{:4}: {}", entry.count, entry.command);
    }

    Ok(())
}

fn load_statistics(path: &Path) -> Fallible<Statistics> {
    let statistics_file = File::open(&path)?;
    let mut buf = BufReader::new(statistics_file);
    let mut statistics_data = Vec::new();
    buf.read_to_end(&mut statistics_data)?;
    Ok(toml::from_slice(&statistics_data)?)
}

fn store_statistics(path: &Path, statistics: &Statistics) -> Fallible<()> {
    use std::fs;
    let data_dir: &Path = path.parent().ok_or_err()?;
    if !data_dir.exists() {
        fs::create_dir_all(data_dir)?;
    }
    let file = File::create(&path)?;
    let mut writer = BufWriter::new(file);
    let statistics_data = toml::to_vec(&statistics)?;
    writer.write_all(&statistics_data)?;
    Ok(())
}

fn increment_command_count(statistics: &mut Statistics, command: &str) {
    match statistics.find_command(command) {
        Some(index) => statistics.entries[index].count += 1,
        None => statistics.entries.push(Entry::new(command)),
    }
}
