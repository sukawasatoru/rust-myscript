/*
 * Copyright 2025, 2026 sukawasatoru
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
use mdns_sd::{IfKind, ServiceEvent};
use reqwest::header;
use rust_myscript::feature::otel::init_otel;
use rust_myscript::prelude::*;
use serde_json::json;
use std::fmt::Write;
use std::time::Duration;
use url::Url;

#[derive(Parser)]
struct Opt {
    #[command(flatten)]
    hue: Option<OptHue>,

    #[command(flatten)]
    remo: Option<OptRemo>,

    #[command(flatten)]
    telegram: Option<OptTelegram>,

    /// Use old service_name for otel
    #[arg(long, env)]
    otel_use_old_service_name: bool,

    /// OpenTelemetry logs endpoint.
    #[arg(long, env)]
    otel_logs_endpoint: Option<Url>,
}

#[derive(Args)]
struct OptHue {
    // use flag instead of the ArgGroup to use Option and flatten.
    /// Use Hue device.
    #[arg(
        long,
        env,
        requires_ifs = [
            (ArgPredicate::IsPresent, "hue_app_key"),
            (ArgPredicate::IsPresent, "hue"),
        ],
    )]
    use_hue: bool,

    /// Application Key for Hue Bridge
    #[arg(long, env, requires = "use_hue")]
    hue_app_key: Option<String>,

    /// Hue Device according `ID=FriendlyName` format
    #[arg(long, value_parser = parse_key_value_arg, requires = "use_hue")]
    hue: Vec<(String, String)>,

    /// timeout for retrieve hue device info
    #[arg(long, env, default_value = "30")]
    timeout_secs: u64,
}

#[derive(Args)]
struct OptRemo {
    // use flag instead of the ArgGroup to use Option and flatten.
    /// Use Hue device.
    #[arg(
        long,
        env,
        requires_ifs = [
            (ArgPredicate::IsPresent, "nature_auth_token"),
            (ArgPredicate::IsPresent, "remo"),
        ],
    )]
    use_remo: bool,

    /// Access token for Nature API
    #[arg(long, env, requires = "use_remo")]
    nature_auth_token: Option<String>,

    /// Remo according `ID=FriendlyName` format
    #[arg(long, value_parser = parse_key_value_arg, requires = "use_remo")]
    remo: Vec<(String, String)>,
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

    /// Template to send message that include `{value}` to insert value
    #[arg(long, env, requires = "use_telegram")]
    telegram_text_template: Option<String>,
}

#[tokio::main]
async fn main() -> Fallible<()> {
    dotenv::dotenv().ok();

    let opt = Opt::parse();

    let client = create_client_builder().build()?;

    let otel_guard = match opt.otel_logs_endpoint {
        Some(endpoint) => {
            let guard = init_otel(
                endpoint,
                env!("CARGO_PKG_NAME"),
                if opt.otel_use_old_service_name {
                    "temperature-remo"
                } else {
                    env!("CARGO_BIN_NAME")
                },
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

    let mut sensor_values: Vec<(String, f64, Option<f64>)> = vec![];

    let mut _failed_hue = false;

    if let Some(hue) = opt.hue {
        match retrieve_hue_temperature(hue).await {
            Ok((list, has_error)) => {
                sensor_values.append(
                    &mut list
                        .into_iter()
                        .map(|(name, temperature)| (name, temperature, None))
                        .collect(),
                );
                _failed_hue = has_error;
            }
            Err(e) => {
                info!(?e, "failed to retrieve temperature from hue");
                _failed_hue = true;
            }
        };
    }

    let mut _failed_remo = false;
    if let Some(remo) = opt.remo {
        let res_devices = client
            .get("https://api.nature.global/1/devices")
            .header(header::ACCEPT, "application/json")
            .bearer_auth(remo.nature_auth_token.expect("--nature-auth-token"))
            .send()
            .await?;
        info!(?res_devices);
        let res_devices_text = res_devices.text().await?;
        debug!(res_devices_text);

        let devices = serde_json::from_str::<serde_json::Value>(&res_devices_text)?;
        for (id, friendly_name) in remo.remo {
            let device = get_device(&devices, &id)?;

            let temperature = get_temperature(device)?;
            let humidity = get_humidity(device)?;

            sensor_values.push((friendly_name, temperature, Some(humidity)));
        }
    }

    ensure!(!sensor_values.is_empty(), "no reports");

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
                &sensor_values,
            )?)
            .send()
            .await?;
        info!(?ret_telegram);
        debug!(ret_telegram_text = %ret_telegram.text().await?);
    }

    for (name, temperature, humidity) in &sensor_values {
        println!(
            "{}:\n  temperature: {}\n  humidity: {}",
            name,
            temperature,
            humidity
                .map(|data| data.to_string())
                .unwrap_or_else(|| "N/A".into()),
        );

        if otel_guard.is_some() {
            info!(
                event.name = "device.app.result",
                device.name = name,
                temperature,
                humidity,
            );
        }
    }

    Ok(())
}

fn create_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().user_agent(concat!(
        env!("CARGO_PKG_NAME"),
        " (https://github.com/sukawasatoru/rust-myscript/)",
    ))
}

#[tracing::instrument(skip_all)]
async fn retrieve_hue_temperature(hue: OptHue) -> Fallible<(Vec<(String, f64)>, bool)> {
    let mdns = mdns_sd::ServiceDaemon::new().context("failed to create mdns daemon")?;
    let receiver = mdns.browse("_hue._tcp.local.")?;

    mdns.disable_interface(IfKind::IPv6)?;
    let client = create_client_builder()
        .danger_accept_invalid_certs(true)
        .build()?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let mut devices = hue.hue.clone();

    tokio::task::spawn(tokio::time::timeout(
        Duration::from_secs(hue.timeout_secs),
        async move {
            while let Ok(event) = receiver.recv_async().await {
                let service_info = match event {
                    ServiceEvent::ServiceResolved(service_info) => {
                        info!(?service_info, "ServiceResolved");
                        service_info
                    }
                    event => {
                        debug!(?event);
                        continue;
                    }
                };

                let addresses = service_info.get_addresses_v4();
                let address = match addresses.iter().next() {
                    Some(data) => data,
                    None => continue,
                };
                let res = client
                    .get(format!("https://{address}/clip/v2/resource/temperature"))
                    .header(
                        "hue-application-key",
                        hue.hue_app_key.as_ref().expect("--hue-app-key"),
                    )
                    .send()
                    .await;
                let res_body = match res {
                    Ok(data) => match data.text().await {
                        Ok(data) => data,
                        Err(e) => {
                            info!(?e, "failed to retrieve response");
                            continue;
                        }
                    },
                    Err(e) => {
                        info!(?e, "failed to sending request");
                        continue;
                    }
                };

                debug!(res_body);
                let res_json = match serde_json::from_str::<serde_json::Value>(&res_body) {
                    Ok(data) => data,
                    Err(e) => {
                        info!(?e, "failed to parse response");
                        continue;
                    }
                };

                /// https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_temperature
                fn parse_res_json(res: &serde_json::Value) -> Fallible<Vec<(String, f64)>> {
                    let data = res["data"].as_array().context(".data.as_array()")?;
                    let mut list = Vec::with_capacity(data.len());
                    for entry in data {
                        let id = entry["id"]
                            .as_str()
                            .map(ToOwned::to_owned)
                            .context(".data[].id")?;
                        let temperature = entry["temperature"]["temperature_report"]["temperature"]
                            .as_f64()
                            .context(".data[].temperature.temperature_report.temperature")?;
                        list.push((id, temperature));
                    }
                    Ok(list)
                }
                let temperatures = match parse_res_json(&res_json) {
                    Ok(data) => data,
                    Err(e) => {
                        info!(?e, "failed to collect temperature");
                        continue;
                    }
                };

                for (id, temperature) in temperatures {
                    let index = match devices.iter().position(|(data, _)| &id == data) {
                        Some(data) => data,
                        None => continue,
                    };
                    let (_, friendly_name) = devices.swap_remove(index);

                    if tx.send((friendly_name, temperature)).await.is_err() {
                        return;
                    }

                    if devices.is_empty() {
                        return;
                    }
                }
            }
        },
    ));

    let mut result = vec![];
    while let Some(data) = rx.recv().await {
        result.push(data);
    }

    let has_error = hue.hue.len() != result.len();

    while let Err(mdns_sd::Error::Again) = mdns.shutdown() {
        debug!("retry shutting down");
    }

    Ok((result, has_error))
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
    values: &[(String, f64, Option<f64>)],
) -> Fallible<String> {
    let mut report = String::new();
    for (i, (friendly_name, temperature, humidity)) in values.iter().enumerate() {
        write!(report, "*{friendly_name}*\n  温度：{temperature}℃")?;
        if let Some(humidity) = humidity {
            write!(report, "\n  湿度：{humidity}%")?;
        }
        if i != values.len() - 1 {
            report.write_str("\n")?;
        }
    }
    // escape for MarkdownV2.
    let report = report.replace('.', r"\.");

    let reg = regex::Regex::new(r#"([_\[\]()~`>#+=\-|{}.!])"#)?;

    // `\n` to new line for serde_json::json.
    let text = template_txt.replace(r"\n", "\n");
    let text = reg
        .replace_all(&text, r#"\$1"#)
        .replace(r"\{value\}", &report);

    info!(%text);
    let payload = json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "MarkdownV2",
    });
    Ok(serde_json::to_string(&payload)?)
}

const PARSE_KEY_VALUE_ARG_ERR_MSG: &str = "format should be the following: <uuid>:<friendly name>";

fn parse_key_value_arg(value: &str) -> Result<(String, String), &'static str> {
    let mut iter = value.split("=");
    let key = iter.next().ok_or(PARSE_KEY_VALUE_ARG_ERR_MSG)?.to_owned();
    let value = iter.next().ok_or(PARSE_KEY_VALUE_ARG_ERR_MSG)?.to_owned();

    if iter.next().is_some() {
        return Err(PARSE_KEY_VALUE_ARG_ERR_MSG);
    }

    Ok((key, value))
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
    fn opt_hue_ok() {
        let opt = Opt::try_parse_from([
            "temperature-remo",
            "--use-hue",
            "--hue-app-key",
            "app-key",
            "--hue",
            "key=value",
            "--hue",
            "key2=value2",
        ])
        .unwrap();

        assert_eq!(
            opt.hue.as_ref().unwrap().hue[0],
            ("key".into(), "value".into()),
        );
        assert_eq!(
            opt.hue.as_ref().unwrap().hue[1],
            ("key2".into(), "value2".into()),
        );
    }

    #[test]
    fn opt_hue_missing_use_hue() {
        let opt = Opt::try_parse_from(["temperature-remo", "--hue", "key=value"]);

        assert!(opt.is_err());
    }

    #[test]
    fn opt_hue_invalid_format() {
        let opt = Opt::try_parse_from(["temperature-remo", "--use-hue", "--hue", "key"]);

        assert!(opt.is_err());
    }

    #[test]
    fn opt_telegram_ok() {
        Opt::try_parse_from([
            "temperature-remo",
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
