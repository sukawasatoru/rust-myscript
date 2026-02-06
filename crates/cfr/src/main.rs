use std::env::Args;
use std::path::Path;

fn main() {
    let mut command = std::process::Command::new("java");

    let jar_path = ["/usr/local/share/cfr/cfr.jar", "/usr/share/cfr/cfr.jar"]
        .iter()
        .find(|data| {
            let p = Path::new(data);
            p.exists()
        })
        .unwrap_or_else(|| {
            eprintln!("cfr not found");
            std::process::exit(1);
        });

    command
        .arg("-jar")
        .arg(jar_path)
        .args(get_args())
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .output()
        .unwrap();
}

fn get_args() -> Args {
    let mut args = std::env::args();
    args.next();
    args
}
