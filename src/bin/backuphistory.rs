//! #!/bin/bash -eu
//!
//! cp ~/.bash_history .
//! git add .
//! git commit -m "update"

extern crate structopt;

use std::{path::PathBuf, process::Command};

use dotenv;
use env_logger;
use log::{debug, info};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "backuphistory")]
struct Config {
    #[structopt(
        short = "s",
        long = "source",
        help = "e.g. -s ~/.bash_history",
        parse(from_os_str)
    )]
    source: PathBuf,

    #[structopt(
        short = "t",
        long = "target",
        help = "e.g. -t ~/git-repo.git",
        parse(from_os_str)
    )]
    target: PathBuf,

    #[structopt(short = "m", long = "message", help = "e.g. -m commit_message")]
    message: Option<String>,
}

fn main() {
    dotenv::dotenv().ok();
    env_logger::init();

    info!("Hello");

    let config: Config = Config::from_args();
    debug!("config={:?}", config);
    let source = config.source;
    let target = config.target;
    if !source.exists() {
        println!("failed to resolve source: {:?}", target);
        return;
    }

    if !target.exists() {
        println!("failed to resolve target: {:?}", target);
        return;
    }

    let target = if target.is_dir() {
        let mut target_dir = target.to_owned();
        target_dir.push(source.file_name().unwrap());
        target_dir
    } else {
        target.to_owned()
    };

    debug!("target={:?}", target);

    std::fs::copy(&source, &target).unwrap();

    Command::new("git")
        .current_dir(&target.parent().unwrap())
        .arg("add")
        .arg(".")
        .spawn()
        .expect("failed to add file")
        .wait_with_output()
        .expect("failed to wait to add");

    Command::new("git")
        .current_dir(&target.parent().unwrap())
        .arg("commit")
        .arg("-m")
        .arg(&config.message.unwrap_or_else(|| "update".to_string()))
        .spawn()
        .expect("failed to commit")
        .wait_with_output()
        .expect("failed to wait to commit");

    info!("Bye");
}
