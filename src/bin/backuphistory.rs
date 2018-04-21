//! #!/bin/bash -eu
//!
//! cp ~/.bash_history .
//! git add .
//! git commit -m "update"

extern crate env_logger;
extern crate getopts;
#[macro_use]
extern crate log;

use std::path::Path;
use std::process::Command;

#[derive(Debug)]
struct Config {
    source: String,
    target: String,
    message: Option<String>,
}

fn main() {
    env_logger::init();
    info!("Hello");

    let config = get_config().unwrap();
    debug!("config={:?}", config);
    let source = Path::new(&config.source);
    let target = Path::new(&config.target);
    if !source.exists() {
        println!("failed to resolve source: {:?}", target);
        return;
    }

    if !target.exists() {
        println!("failed to resolve target: {:?}", target);
        return;
    }

    let target = match target.is_dir() {
        true => {
            let mut target_dir = target.to_owned();
            target_dir.push(source.file_name().unwrap());
            target_dir
        }
        false => target.to_owned(),
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
        .arg(&config.message.unwrap_or("update".to_string()))
        .spawn()
        .expect("failed to commit")
        .wait_with_output()
        .expect("failed to wait to commit");

    info!("Bye");
}

fn get_config() -> Result<Config, String> {
    let mut options = getopts::Options::new();
    options.reqopt("s", "source", "", "-s ~/.bash_history")
        .reqopt("t", "targetdir", "", "-t ~/git-repo.git")
        .optopt("m", "message", "", "-m commit_message");

    let args = std::env::args().collect::<Vec<String>>();
    let matches = match options.parse(&args[1..]) {
        Ok(result) => result,
        Err(e) => return Err(e.to_string()),
    };

    Ok(Config {
        source: matches.opt_str("s").unwrap(),
        target: matches.opt_str("t").unwrap(),
        message: matches.opt_str("m"),
    })
}
