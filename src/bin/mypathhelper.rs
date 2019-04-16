use log::{debug, info};

fn main() {
    dotenv::dotenv().ok();
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
