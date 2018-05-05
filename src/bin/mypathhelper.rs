extern crate env_logger;
#[macro_use]
extern crate log;

fn main() {
    env_logger::init();

    info!("Hello");

    let paths = std::env::var("PATH").unwrap();
    debug!("{}", paths);

    let mut dest: Vec<&str> = Vec::new();
    for path in paths.split(':') {
        debug!("path: {}", path);
        if !dest.contains(&path) {
            debug!("append: {}", path);
            dest.push(&path);
        }
    }

    println!("{}", &dest.join(":"));

    info!("Bye");
}
