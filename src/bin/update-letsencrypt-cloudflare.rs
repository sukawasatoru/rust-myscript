use reqwest::header::HeaderValue;
use rust_myscript::myscript::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::prelude::*;
use std::io::{BufReader, BufWriter};

#[derive(Default, Deserialize, Serialize)]
struct Config {
    cloudflare_zone_identifier: String,
    cloudflare_api_token: String,
}

fn main() -> anyhow::Result<()> {
    let dirs = directories::ProjectDirs::from("jp", "tinyport", "updateletsencryptcloudflare")
        .context("no valid home directory path")?;
    let config_dir = dirs.config_dir();

    let config_file_path = config_dir.join("config.toml");
    if !config_dir.exists() {
        std::fs::create_dir_all(config_dir).context("failed to create config directory")?;
        let mut writer = BufWriter::new(std::fs::File::create(&config_file_path)?);
        let empty_config: Config = Default::default();
        writer.write_all(toml::to_string(&empty_config)?.as_bytes())?;
        anyhow::bail!(
            "please set preferences to {}",
            config_file_path.to_str().context("no valid unicode")?
        );
    }

    let mut reader = BufReader::new(std::fs::File::open(&config_file_path)?);
    let mut config_str = String::new();
    reader.read_to_string(&mut config_str)?;

    let config = toml::from_str::<Config>(&config_str).context("failed to parser a config file")?;

    let domain = std::env::var("CERTBOT_DOMAIN")?;
    let validation_string = std::env::var("CERTBOT_VALIDATION")?;

    let zone_identifier = &config.cloudflare_zone_identifier;
    let api_token = &config.cloudflare_api_token;

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );

    let client = reqwest::blocking::ClientBuilder::new()
        .default_headers(headers)
        .build()?;

    let dns_records_response = client
        .get(&format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records",
            zone_identifier
        ))
        .bearer_auth(api_token)
        .send()?
        .json::<serde_json::Value>()?;

    let select_domain = format!("_acme-challenge.{}", domain);
    let dns_record_identifier = dns_records_response["result"]
        .as_array()
        .context("result is not array")?
        .iter()
        .find_map(|data| {
            data["name"]
                .as_str()
                .filter(|data| data == &select_domain)
                .and_then(|_| data["id"].as_str().map(ToOwned::to_owned))
        })
        .with_context(|| format!("the domain {} is not found", select_domain))?;

    let patch_dns_record_response = client
        .patch(&format!(
            "https://api.cloudflare.com/client/v4/zones/{}/dns_records/{}",
            zone_identifier, dns_record_identifier
        ))
        .bearer_auth(api_token)
        .body(format!(r#"{{"content": "{}"}}"#, validation_string))
        .send()?;

    let status = patch_dns_record_response.status();
    let ret_text = patch_dns_record_response.text()?;

    if status != reqwest::StatusCode::OK {
        anyhow::bail!("failed to update record: {}", ret_text);
    }

    for retry_second in [1, 3, 5, 7, 11, 13, 17, 19, 23, 29].iter() {
        let txt_u8 = trust_dns_resolver::Resolver::default()?
            .txt_lookup(&select_domain)?
            .iter()
            .next()
            .context("failed to lookup txt")?
            .txt_data()
            .iter()
            .map(|data| data.to_vec())
            .next()
            .context("lookup response is empty")?;

        let txt_string = String::from_utf8(txt_u8)?;
        println!("txt_string: {}", txt_string);
        if txt_string == validation_string {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(*retry_second));
    }

    Ok(())
}
