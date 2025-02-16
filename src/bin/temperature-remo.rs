/*
 * Copyright 2025 sukawasatoru
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use clap::builder::ArgPredicate;
use clap::{Args, Parser};
use reqwest::header;
use rust_myscript::feature::otel::init_otel;
use rust_myscript::prelude::*;
use serde_json::json;
use url::Url;

#[derive(Parser)]
struct Opt {
    /// Access token for Nature API
    #[arg(long, env)]
    nature_auth_token: String,

    /// Device ID to retrieve temperature
    #[arg(long, env)]
    remo_id: String,

    #[command(flatten)]
    telegram: Option<OptTelegram>,

    /// OpenTelemetry logs endpoint.
    #[arg(long, env)]
    otel_logs_endpoint: Option<Url>,
}

#[derive(Args)]
struct OptTelegram {
    // use flag instead of the ArgGroup to use Option and flatten.
    /// Notify to Telegram
    #[arg(
        long,
        env,
        requires_ifs = [
            (ArgPredicate::IsPresent, "telegram_bot_token"),
            (ArgPredicate::IsPresent, "telegram_chat_id"),
            (ArgPredicate::IsPresent, "telegram_text_template"),
        ],
    )]
    use_telegram: bool,

    /// Authorization token to use Bot.
    #[arg(long, env, requires = "use_telegram")]
    telegram_bot_token: Option<String>,

    /// Chat ID to notify to Telegram
    #[arg(long, env, requires = "use_telegram")]
    telegram_chat_id: Option<String>,

    /// Template to send message that include `{temperature}`/`{humidity}` to insert value
    #[arg(long, env, requires = "use_telegram")]
    telegram_text_template: Option<String>,
}

#[tokio::main]
async fn main() -> Fallible<()> {
    dotenv::dotenv().ok();

    let opt = Opt::parse();

    let client = reqwest::Client::builder()
        .user_agent("temperature-remo (https://github.com/sukawasatoru/rust-myscript/)")
        .build()?;

    let otel_guard = match opt.otel_logs_endpoint {
        Some(endpoint) => {
            let guard = init_otel(
                client.clone(),
                endpoint,
                env!("CARGO_PKG_NAME"),
                env!("CARGO_BIN_NAME"),
            )?;
            Some(guard)
        }
        None => {
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .init();
            None
        }
    };

    let res_devices = client
        .get("https://api.nature.global/1/devices")
        .header(header::ACCEPT, "application/json")
        .bearer_auth(&opt.nature_auth_token)
        .send()
        .await?;
    info!(?res_devices);
    let res_devices_text = res_devices.text().await?;
    debug!(res_devices_text);

    let devices = serde_json::from_str::<serde_json::Value>(&res_devices_text)?;
    let device = get_device(&devices, &opt.remo_id)?;

    let temperature = get_temperature(device)?;
    let humidity = get_humidity(device)?;

    println!("temperature: {}\nhumidity: {}", temperature, humidity);

    if let Some(telegram) = opt.telegram {
        info!("notify to telegram");
        let ret_telegram = client
            .post(format!(
                "https://api.telegram.org/bot{}/sendMessage",
                telegram
                    .telegram_bot_token
                    .expect("telegram_bot_token should not be None"),
            ))
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/json")
            .body(generate_telegram_payload(
                &telegram
                    .telegram_chat_id
                    .expect("telegram_chat_id should not be None"),
                &telegram
                    .telegram_text_template
                    .expect("telegram_text_template should not be None"),
                temperature,
                humidity,
            )?)
            .send()
            .await?;
        info!(?ret_telegram);
        debug!(ret_telegram_text = %ret_telegram.text().await?);
    }

    if otel_guard.is_some() {
        info!(event.name = "device.app.result", temperature, humidity);
    }

    Ok(())
}

fn get_device<'a>(res: &'a serde_json::Value, remo_id: &str) -> Fallible<&'a serde_json::Value> {
    res.as_array()
        .expect(". should be array")
        .iter()
        .find(|data| data.as_object().expect(".[]. should be object")["id"] == remo_id)
        .with_context(|| format!("remo {} is not found", remo_id))
}

fn get_temperature(device: &serde_json::Value) -> Fallible<f64> {
    device["newest_events"]
        .get("te")
        .context("temperature event is not exist")?["val"]
        .as_f64()
        .context("temperature")
}

fn get_humidity(device: &serde_json::Value) -> Fallible<f64> {
    device["newest_events"]
        .get("hu")
        .context("temperature event is not exist")?["val"]
        .as_f64()
        .context("humidity")
}

fn generate_telegram_payload(
    chat_id: &str,
    template_txt: &str,
    temperature: f64,
    humidity: f64,
) -> Fallible<String> {
    let text = template_txt
        .replace(
            "{temperature}",
            &format!("{temperature}℃").replace('.', r"\."),
        )
        .replace("{humidity}", &format!("{humidity}%").replace('.', r"\."))
        // `\n` to new line.
        .replace(r#"\n"#, "\n");

    info!(%template_txt);
    info!(%text);
    let payload = json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "MarkdownV2",
    });
    Ok(serde_json::to_string(&payload)?)
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
    fn opt_telegram_ok() {
        Opt::try_parse_from([
            "temperature-remo",
            "--nature-auth-token",
            "nature-auth-token",
            "--remo-id",
            "remo-id",
            "--use-telegram",
            "--telegram-bot-token",
            "token",
            "--telegram-chat-id",
            "chat-id",
            "--telegram-text-template",
            "text-template",
        ])
        .unwrap();
    }

    #[test]
    fn opt_telegram_missing_use_telegram() {
        let opt = Opt::try_parse_from([
            "temperature-remo",
            "--nature-auth-token",
            "nature-auth-token",
            "--remo-id",
            "remo-id",
            "--telegram-bot-token",
            "token",
            "--telegram-chat-id",
            "chat-id",
            "--telegram-text-template",
            "text-template",
        ]);
        assert!(opt.is_err());
    }

    #[test]
    fn opt_telegram_missing_telegram_bot_token() {
        let opt = Opt::try_parse_from([
            "temperature-remo",
            "--nature-auth-token",
            "nature-auth-token",
            "--remo-id",
            "remo-id",
            "--use-telegram",
            "--telegram-chat-id",
            "chat-id",
            "--telegram-text-template",
            "text-template",
        ]);
        assert!(opt.is_err());
    }

    #[test]
    fn opt_telegram_missing_telegram_chat_id() {
        let opt = Opt::try_parse_from([
            "temperature-remo",
            "--nature-auth-token",
            "nature-auth-token",
            "--remo-id",
            "remo-id",
            "--use-telegram",
            "--telegram-bot-token",
            "token",
            "--telegram-text-template",
            "text-template",
        ]);
        assert!(opt.is_err());
    }

    #[test]
    fn opt_telegram_missing_telegram_text_template() {
        let opt = Opt::try_parse_from([
            "temperature-remo",
            "--nature-auth-token",
            "nature-auth-token",
            "--remo-id",
            "remo-id",
            "--use-telegram",
            "--telegram-bot-token",
            "token",
            "--telegram-chat-id",
            "chat-id",
        ]);
        assert!(opt.is_err());
    }

    #[test]
    fn get_device_ok() {
        let devices = serde_json::from_str(TEST_RES).unwrap();
        let device = get_device(&devices, "d02b1856-e29f-42a0-bd73-08498d706466").unwrap();
        assert_eq!(
            device["id"].as_str().unwrap(),
            "d02b1856-e29f-42a0-bd73-08498d706466",
        );
    }

    #[test]
    fn get_temperature_ok() {
        let devices = serde_json::from_str(TEST_RES).unwrap();
        let device = get_device(&devices, "d02b1856-e29f-42a0-bd73-08498d706466").unwrap();
        let actual = get_temperature(device).unwrap();
        assert_eq!(actual, 25.6);
    }

    #[test]
    fn get_humidity_ok() {
        let devices = serde_json::from_str(TEST_RES).unwrap();
        let device = get_device(&devices, "d02b1856-e29f-42a0-bd73-08498d706466").unwrap();
        let actual = get_humidity(device).unwrap();
        assert_eq!(actual, 41f64);
    }

    const TEST_RES: &str = r#"
[
  {
    "name": "foo room",
    "id": "1f99c86d-bdad-4199-8225-0d4ac80cfb2b",
    "created_at": "2022-12-26T05:03:15Z",
    "updated_at": "2022-12-29T04:01:30Z",
    "mac_address": "00:00:00:00:00:00",
    "serial_number": "serial",
    "firmware_version": "Remo-mini/2.0.62-gf5b5d27",
    "temperature_offset": 0,
    "humidity_offset": 0,
    "users": [
      {
        "id": "9feb4339-058f-4c04-a5f1-ecb164833ad1",
        "nickname": "piyo",
        "superuser": true
      },
      {
        "id": "fb326ae0-877a-4bee-92a1-b9ccab2100e8",
        "nickname": "hoge",
        "superuser": false
      }
    ],
    "newest_events": {
      "te": {
        "val": 16.8,
        "created_at": "2025-01-05T07:25:20Z"
      }
    }
  },
  {
    "name": "bar living",
    "id": "d02b1856-e29f-42a0-bd73-08498d706466",
    "created_at": "2021-08-15T13:59:03Z",
    "updated_at": "2024-11-09T18:30:15Z",
    "mac_address": "00:00:00:00:00:00",
    "bt_mac_address": "00:00:00:00:00:00",
    "serial_number": "serial",
    "firmware_version": "Remo/1.14.8",
    "temperature_offset": 0,
    "humidity_offset": 0,
    "users": [
      {
        "id": "9feb4339-058f-4c04-a5f1-ecb164833ad1",
        "nickname": "piyo",
        "superuser": true
      }
    ],
    "newest_events": {
      "hu": {
        "val": 41,
        "created_at": "2025-02-15T17:39:14Z"
      },
      "il": {
        "val": 0,
        "created_at": "2025-02-15T17:48:53Z"
      },
      "mo": {
        "val": 1,
        "created_at": "2025-02-12T03:46:00Z"
      },
      "te": {
        "val": 25.6,
        "created_at": "2025-02-15T17:49:15Z"
      }
    },
    "online": true
  },
  {
    "name": "foo living",
    "id": "ad21c513-7eef-4651-b02d-a6e28ca11a15",
    "created_at": "2018-05-06T06:39:37Z",
    "updated_at": "2024-08-15T04:02:34Z",
    "mac_address": "00:00:00:00:00:00",
    "serial_number": "serial",
    "firmware_version": "Remo/1.0.69-gbbcc0de",
    "temperature_offset": 1,
    "humidity_offset": -20,
    "users": [
      {
        "id": "9feb4339-058f-4c04-a5f1-ecb164833ad1",
        "nickname": "piyo",
        "superuser": true
      },
      {
        "id": "fb326ae0-877a-4bee-92a1-b9ccab2100e8",
        "nickname": "hoge",
        "superuser": false
      }
    ],
    "newest_events": {
      "hu": {
        "val": 39,
        "created_at": "2025-02-15T17:49:42Z"
      },
      "il": {
        "val": 3,
        "created_at": "2025-02-15T13:59:41Z"
      },
      "mo": {
        "val": 1,
        "created_at": "2025-02-15T13:57:45Z"
      },
      "te": {
        "val": 16,
        "created_at": "2025-02-15T17:49:42Z"
      }
    }
  }
]
"#;
}
