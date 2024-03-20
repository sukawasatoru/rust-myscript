use clap::{Parser, Subcommand};
use rust_myscript::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::{create_dir_all, File};
use std::io::prelude::*;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;

#[derive(Debug, Parser)]
struct Opt {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(name = "trim")]
    Trim {
        /// Backup a FILE to specified path
        #[arg(short, long = "backup")]
        backup_path: Option<PathBuf>,

        /// history file
        #[arg(name = "FILE")]
        history_path: PathBuf,
    },
    #[command(name = "show")]
    Show {
        /// prints the first NUM lines
        #[arg(name = "NUM", short, long = "lines")]
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
    fn increment_command_count(&mut self, command: &str) {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.command == command);
        match entry {
            Some(entry) => entry.count += 1,
            None => self.entries.push(Entry::new(command)),
        }
    }
}

fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let opt: Opt = Opt::parse();
    debug!(?opt);

    match opt.cmd {
        Command::Trim {
            backup_path,
            history_path,
        } => trim_from_path(history_path, backup_path, None),
        Command::Show { num } => show(num),
    }
}

#[tracing::instrument]
fn trim_from_path(
    history_path: PathBuf,
    backup_path: Option<PathBuf>,
    statistics_path: Option<PathBuf>,
) -> Fallible<()> {
    let project_dirs =
        directories::ProjectDirs::from("jp", "tinyport", "trimhistory").context("ProjectDirs")?;
    let statistics_path =
        statistics_path.unwrap_or_else(|| project_dirs.data_dir().join("statistics.toml"));

    statistics_path
        .parent()
        .context("statistics parent")
        .and_then(|parent| {
            if !parent.exists() {
                create_dir_all(parent).context("create statistics directory")
            } else {
                Ok(())
            }
        })?;

    let backup_dest = match backup_path {
        Some(dest) => Some(BufWriter::new(File::create(dest)?)),
        None => None,
    };

    trim(
        (BufReader::new(File::open(&history_path)?), || {
            Ok(BufWriter::new(File::create(&history_path)?))
        }),
        backup_dest,
        (BufReader::new(File::open(&statistics_path)?), || {
            Ok(BufWriter::new(File::create(&statistics_path)?))
        }),
    )
}

fn trim<
    HistorySrc,
    HistoryDest,
    HistoryDestProvider,
    BackupDest,
    StatisticsSrc,
    StatisticsDest,
    StatisticsDestProvider,
>(
    history: (HistorySrc, HistoryDestProvider),
    mut backup_dest: Option<BackupDest>,
    statistics: (StatisticsSrc, StatisticsDestProvider),
) -> Fallible<()>
where
    HistorySrc: BufRead,
    HistoryDest: Write,
    HistoryDestProvider: FnOnce() -> Fallible<HistoryDest>,
    BackupDest: Write,
    StatisticsSrc: Read,
    StatisticsDest: Write,
    StatisticsDestProvider: FnOnce() -> Fallible<StatisticsDest>,
{
    let (mut history_src, history_dest_provider) = history;
    let (statistics_src, statistics_dest_provider) = statistics;

    let mut statistics = load_statistics(statistics_src)?;

    let mut line = String::new();
    let mut trimmed = Vec::new();
    let mut trim_count = 0;
    loop {
        match history_src.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                if let Some(ref mut backup_dest) = backup_dest {
                    write!(backup_dest, "{line}")?;
                }

                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                debug!(result = ?line);
                let trimmed_line = line.trim();
                if let Some(index) = trimmed.iter().position(|entity| trimmed_line == entity) {
                    debug!(index, "contains");
                    trimmed.remove(index);
                    trim_count += 1;
                    statistics.increment_command_count(trimmed_line);
                }
                trimmed.push(trimmed_line.to_owned());
                line.clear();
            }
            Err(e) => return Err(e.into()),
        }
    }

    if let Some(ref mut backup_dest) = backup_dest {
        backup_dest.flush()?;
    }

    info!(trim_count, trimmed_len = trimmed.len());

    let mut history_dest = history_dest_provider()?;
    for entity in trimmed.iter() {
        writeln!(&mut history_dest, "{entity}")?;
    }
    history_dest.flush()?;

    store_statistics(statistics_dest_provider()?, &statistics)?;

    Ok(())
}

fn show(num: Option<i32>) -> Fallible<()> {
    let project_dirs =
        directories::ProjectDirs::from("jp", "tinyport", "trimhistory").context("ProjectDirs")?;
    let statistics_path = project_dirs.data_dir().join("statistics.toml");

    let mut statistics: Statistics = load_statistics(BufReader::new(File::open(statistics_path)?))?;

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

fn load_statistics<R: Read>(mut src: R) -> Fallible<Statistics> {
    let mut buf = String::new();
    src.read_to_string(&mut buf)?;
    toml::from_str(&buf).context("failed to parse statistics")
}

fn store_statistics<Dest: Write>(mut dest: Dest, statistics: &Statistics) -> Fallible<()> {
    let statistics_data = toml::to_string(statistics)?;
    dest.write_all(statistics_data.as_bytes())?;
    dest.flush().context("failed to flush statistics")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Opt::command().debug_assert();
    }

    #[test]
    fn test_trim() {
        let history_src = r#"
pwd
pwd
ls
ls -l
cd ~/
pwd
cd ~/
"#;

        let statistics_src = r#"
[[entries]]
command = "pwd"
count = 4113

[[entries]]
command = "ls -l"
count = 416

[[entries]]
command = "ls -la"
count = 317
"#;

        let mut actual_history = Vec::new();
        let history_writer = BufWriter::new(&mut actual_history);

        let mut actual_backup = Vec::new();
        let backup_writer = BufWriter::new(&mut actual_backup);

        let mut actual_statistics = Vec::new();
        let statistics_writer = BufWriter::new(&mut actual_statistics);

        trim(
            (history_src.as_bytes(), || Ok(history_writer)),
            Some(backup_writer),
            (statistics_src.as_bytes(), || Ok(statistics_writer)),
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(actual_history).unwrap(),
            r#"
ls
ls -l
pwd
cd ~/
"#
        );

        assert_eq!(String::from_utf8(actual_backup).unwrap(), history_src);

        assert_eq!(
            String::from_utf8(actual_statistics).unwrap(),
            r#"[[entries]]
command = "pwd"
count = 4115

[[entries]]
command = "ls -l"
count = 416

[[entries]]
command = "ls -la"
count = 317

[[entries]]
command = "cd ~/"
count = 1
"#
        );
    }
}
