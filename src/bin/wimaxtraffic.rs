use std::{
    fs::{self, File},
    io::{prelude::*, BufWriter},
    path::Path,
};

use log::info;
use serde_derive::{Deserialize, Serialize};

use rust_myscript::myscript::prelude::*;

#[derive(Deserialize, Serialize)]
struct Config {
    host: Option<String>,
    token: Option<String>,
}

impl Config {
    fn new() -> Config {
        Default::default()
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            host: Default::default(),
            token: Default::default(),
        }
    }
}

fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    env_logger::init();
    let project_dir =
        directories::ProjectDirs::from("jp", "tinyport", "wimaxtraffic").ok_or_err()?;
    let config_path = project_dir.config_dir().join("config.toml");
    let mut loader = TomlLoader::new();
    let config = prepare_config(&mut loader, &config_path)?;
    let host = get_wimax_host(&config).expect("need host");
    let token = get_wx04_token(&config).expect("need WX04 token");
    let body = reqwest::Client::new()
        .get(&format!("http://{}/index.cgi/network_count_main", host))
        .header(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&format!("Basic {}", token))?,
        )
        .send()?
        .text()?;
    let reg = regex::Regex::new(r#"[0-9]*\.[0-9]*[bBMG][^<]*"#)?;
    let counters = reg.captures_iter(&body).collect::<Vec<_>>();
    println!(
        "3days until the today and remaining: {}",
        counters[1]
            .get(0)
            .expect("3days until the today and remaining")
            .as_str()
    );
    println!(
        "3days until the previous day: {}",
        counters[2]
            .get(0)
            .expect("3days until the previous day")
            .as_str()
    );
    println!("today: {}", counters[3].get(0).expect("today").as_str());
    println!(
        "1day ago: {}",
        counters[4].get(0).expect("1day ago").as_str()
    );
    println!(
        "2day ago: {}",
        counters[5].get(0).expect("2day ago").as_str()
    );
    println!(
        "3day ago: {}",
        counters[6].get(0).expect("3day ago").as_str()
    );
    Ok(())
}

fn prepare_config(loader: &mut TomlLoader, path: &Path) -> Fallible<Config> {
    if path.exists() {
        return loader.load(path);
    }

    info!("create new config file");
    let dir = path.parent().ok_or_err()? as &Path;
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }

    let config = Config::new();
    let mut buffer = BufWriter::new(File::create(path)?);
    buffer.write_all(&toml::to_vec(&config)?)?;
    info!("Config file created successfully: {:?}", path);
    Ok(config)
}

fn get_wimax_host(config: &Config) -> Option<String> {
    config.host.clone()
}

fn get_wx04_token(config: &Config) -> Option<String> {
    std::env::var("WX04TOKEN")
        .ok()
        .or_else(|| config.token.clone())
}
