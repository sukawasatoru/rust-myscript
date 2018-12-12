use std::env::Args;

use dotenv;
use env_logger;
use log::info;

fn main() {
    dotenv::dotenv().ok();
    env_logger::init();

    info!("Hello");

    let mut command = std::process::Command::new("java");

    command
        .arg("-jar")
        .arg(std::env::var("HOME").unwrap() + "/bin/cfr_0_115.jar")
        .args(get_args())
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .output()
        .unwrap();

    info!("Bye");
}

fn get_args() -> Args {
    let mut args = std::env::args();
    args.next();
    args
}
