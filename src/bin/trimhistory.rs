extern crate dotenv;
extern crate env_logger;
extern crate getopts;
#[macro_use]
extern crate log;

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;

use getopts::Options;

#[derive(Debug)]
struct Config {
    help: bool,
    backup_path: Option<PathBuf>,
    history_path: Option<PathBuf>,
}

fn main() {
    use std::env;
    use std::process::exit;

    dotenv::dotenv().ok();
    env_logger::init();

    info!("Hello");

    let options = generate_options();
    let config = parse_options(&options, &env::args().collect::<Vec<_>>());
    debug!("config: {:?}", config);
    if need_to_show_help(&config) {
        print_help(&options, env::current_exe().unwrap().file_stem().unwrap().to_str().unwrap());
        exit(if config.help {
            0
        } else {
            1
        });
    }

    let file_path = match config.history_path {
        Some(path) => path,
        None => unreachable!(),
    };
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

    if let Some(backup_path) = config.backup_path {
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

fn generate_options() -> Options {
    let mut options = getopts::Options::new();
    options.optflag("h", "help", "Show help")
        .optopt("b", "backup", "Backup a FILE to specified path",
                "-b ~/.bash_history$(date +%Y%m%d-%H%M%S)");
    options
}

fn parse_options(options: &Options, args: &[String]) -> Config {
    use std::path::Path;
    let matches = options.parse(&args[1..]).unwrap();

    Config {
        help: matches.opt_present("h"),
        backup_path: matches.opt_str("b")
            .map(|entry| Path::new(&entry).to_path_buf()),
        history_path: matches.free.first()
            .map(|entry| Path::new(entry).to_path_buf()),
    }
}

fn need_to_show_help(config: &Config) -> bool {
    config.help || (config.backup_path.is_none() && config.history_path.is_none())
}

fn print_help(options: &Options, program: &str) {
    println!("{}", options.usage(&format!("Usage: {} [Options] FILE", program)));
}
