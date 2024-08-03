/*
 * Copyright 2022, 2023 sukawasatoru
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

use anyhow::anyhow;
use clap::Parser;
use futures::future::BoxFuture;
use reqwest::{header, StatusCode};
use rust_myscript::feature::otel::init_otel;
use rust_myscript::prelude::*;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_224};
use std::fmt::{Display, Formatter};
use std::io::prelude::*;
use std::rc::Rc;
use url::Url;

#[derive(Deserialize)]
struct SitePreferences {
    sites: Vec<Site>,
}

#[derive(Serialize)]
struct SitePrefsForSerialize {
    sites: Vec<Rc<Site>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Site {
    title: String,
    uri: Url,
    uri_open: Option<Url>,
    check_method: CheckMethod,
    hash: Option<String>,
    last_modified: Option<String>,
    etag: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
enum CheckMethod {
    Head,
    Hash,
}

struct CheckOk {
    updated: bool,
    site: Site,
}

#[derive(Debug)]
struct CheckError {
    site: Site,
    source: anyhow::Error,
}

impl Display for CheckError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "failed to check site: {}", self.site.title)
    }
}

// implement manually for avoiding Site implementation.
impl std::error::Error for CheckError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

/// Update checker for web site
#[derive(Parser)]
struct Opt {
    /// Bot name for notifying to slack
    #[arg(long, env, default_value = "siteupdatechecker")]
    slack_notify_bot_name: String,

    /// Channel ID to notify channel
    #[arg(long, env)]
    slack_notify_channel: String,

    /// Web hooks URL for slack
    #[arg(long, env)]
    slack_notify_url: String,

    /// OpenTelemetry logs endpoint.
    #[arg(long, env)]
    otel_logs_endpoint: Option<Url>,
}

#[tokio::main]
async fn main() -> Fallible<()> {
    let opt: Opt = Opt::parse();

    let client = reqwest::ClientBuilder::new()
        .user_agent("siteupdatechecker")
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
            tracing_subscriber::fmt::init();
            None
        }
    };

    let mut site_prefs_string = String::new();
    std::io::stdin().read_to_string(&mut site_prefs_string)?;
    let site_prefs = toml::from_str::<SitePreferences>(&site_prefs_string)?;

    let mut futs = Vec::<BoxFuture<Result<CheckOk, CheckError>>>::new();
    for site in site_prefs.sites {
        match site.check_method {
            CheckMethod::Head => futs.push(Box::pin(check_site_head(client.clone(), site))),
            CheckMethod::Hash => futs.push(Box::pin(check_site_hash(client.clone(), site))),
        }
    }

    let ret = futures::future::join_all(futs).await;
    let mut new_prefs = Vec::with_capacity(ret.len());
    let mut updated_sites = vec![];
    let mut not_modified_sites = vec![];
    let mut error_sites = vec![];
    for ret_check in ret {
        match ret_check {
            Ok(data) => match data.updated {
                true => {
                    info!("updated: {}", &data.site.title);
                    let site = Rc::new(data.site);
                    new_prefs.push(site.clone());
                    updated_sites.push(site);
                }
                false => {
                    info!("not modified: {}", &data.site.title);
                    let site = Rc::new(data.site);
                    new_prefs.push(site.clone());
                    not_modified_sites.push(site);
                }
            },
            Err(e) => {
                info!(?e.source, "error caused: {}", &e.site.title);
                let site = Rc::new(e.site);
                new_prefs.push(site.clone());
                error_sites.push((site, e.source));
            }
        }
    }

    let mut otel_log_body = String::new();

    println!("# updated:");
    otel_log_body.push_str("updated:\n");
    if updated_sites.is_empty() {
        println!("#   (none)");
        otel_log_body.push_str("  (none)\n");
    } else {
        for site in updated_sites.iter() {
            println!("#   {}", site.title);
            otel_log_body.push_str("  ");
            otel_log_body.push_str(&site.title);
            otel_log_body.push_str("\n    ");
            otel_log_body.push_str(site.uri_open.as_ref().unwrap_or(&site.uri).as_str());
            otel_log_body.push('\n');
        }
    }

    println!("#");
    println!("# not modified:");
    otel_log_body.push_str("not modified:\n");
    if not_modified_sites.is_empty() {
        println!("#   (none)");
        otel_log_body.push_str("  (none)\n");
    } else {
        for site in not_modified_sites.iter() {
            println!("#   {}", site.title);
            otel_log_body.push_str("  ");
            otel_log_body.push_str(&site.title);
            otel_log_body.push('\n');
        }
    }

    println!("#");
    println!("# error:");
    otel_log_body.push_str("error:\n");
    if error_sites.is_empty() {
        println!("#   (none)");
        otel_log_body.push_str("  (none)\n");
    } else {
        for (site, e) in error_sites.iter() {
            println!("#   {}\n#     reason: {}", site.title, &e);
            otel_log_body.push_str("  ");
            otel_log_body.push_str(&site.title);
            otel_log_body.push_str("\n    ");
            otel_log_body.push_str(site.uri_open.as_ref().unwrap_or(&site.uri).as_str());
            otel_log_body.push('\n');
            otel_log_body.push_str("    reason: ");
            otel_log_body.push_str(&e.to_string());
            otel_log_body.push('\n');
        }
    }

    let new_site_prefs = SitePrefsForSerialize { sites: new_prefs };
    let new_prefs_string = toml::to_string(&new_site_prefs)?;
    println!("#");
    println!("# new config:");
    println!("{new_prefs_string}");

    if !updated_sites.is_empty() || !error_sites.is_empty() {
        info!("notify to slack");
        let ret_slack = client
            .post(&opt.slack_notify_url)
            .header(header::CONTENT_TYPE, "application/json")
            .body(generate_slack_payload(
                &opt.slack_notify_bot_name,
                &opt.slack_notify_channel,
                &updated_sites,
                &error_sites,
            )?)
            .send()
            .await?;
        info!(?ret_slack);
        let ret_slack_response_text = ret_slack.text().await?;
        info!(ret_slack_response_text);
    }

    if otel_guard.is_some() {
        info!(
            event.name = "device.app.result",
            has_update = %!updated_sites.is_empty(),
            "{}",
            otel_log_body,
        );
    }

    Ok(())
}

async fn check_site_head(client: reqwest::Client, site: Site) -> Result<CheckOk, CheckError> {
    use reqwest::header::ToStrError;

    let mut headers = header::HeaderMap::new();
    if let Some(ref if_modified_since) = site.last_modified {
        match if_modified_since.parse() {
            Ok(data) => {
                headers.insert(header::IF_MODIFIED_SINCE, data);
            }
            Err(e) => {
                return Err(CheckError {
                    site,
                    source: anyhow!(e).context("failed to parse a date string to a header value"),
                })
            }
        }
    }
    if let Some(ref etag) = site.etag {
        match etag.parse() {
            Ok(data) => {
                headers.insert(header::IF_NONE_MATCH, data);
            }
            Err(e) => {
                return Err(CheckError {
                    site,
                    source: anyhow!(e).context("failed to parse a etag string to a header value"),
                })
            }
        }
    }

    let response = match client.head(site.uri.as_str()).headers(headers).send().await {
        Ok(data) => data,
        Err(e) => {
            return Err(CheckError {
                site,
                source: anyhow!(e).context("failed to send request"),
            })
        }
    };

    let get_header_value = |response: &reqwest::Response,
                            key: &header::HeaderName|
     -> Result<Option<String>, ToStrError> {
        response
            .headers()
            .get(key)
            .map(|data| data.to_str())
            .map_or(Ok(None), |data| data.map(|data| Some(data.to_string())))
    };

    let status_code = response.status();

    match status_code {
        StatusCode::OK => Ok(CheckOk {
            updated: true,
            site: Site {
                last_modified: match get_header_value(&response, &header::LAST_MODIFIED) {
                    Ok(data) => data,
                    Err(e) => {
                        return Err(CheckError {
                            site,
                            source: anyhow!(e).context("failed to parse a date to string (200)"),
                        })
                    }
                },
                etag: match get_header_value(&response, &header::ETAG) {
                    Ok(data) => data,
                    Err(e) => {
                        return Err(CheckError {
                            site,
                            source: anyhow!(e).context("failed to parse a etag to string (200)"),
                        })
                    }
                },
                ..site
            },
        }),
        StatusCode::NOT_MODIFIED => Ok(CheckOk {
            updated: false,
            site: Site {
                last_modified: match get_header_value(&response, &header::LAST_MODIFIED) {
                    Ok(data) => data,
                    Err(e) => {
                        return Err(CheckError {
                            site,
                            source: anyhow!(e).context("failed to parse a date to string (304)"),
                        })
                    }
                },
                etag: match get_header_value(&response, &header::ETAG) {
                    Ok(data) => data,
                    Err(e) => {
                        return Err(CheckError {
                            site,
                            source: anyhow!(e).context("failed to parse a etag to string (304)"),
                        })
                    }
                },
                ..site
            },
        }),
        _ => Err(CheckError {
            site,
            source: anyhow!("unexpected status code: {}", status_code.as_u16()),
        }),
    }
}

async fn check_site_hash(client: reqwest::Client, site: Site) -> Result<CheckOk, CheckError> {
    let response = match client.get(site.uri.as_str()).send().await {
        Ok(data) => data,
        Err(e) => {
            return Err(CheckError {
                site,
                source: anyhow!(e).context("failed to send request"),
            });
        }
    };

    let status_code = response.status();
    if status_code != StatusCode::OK {
        return Err(CheckError {
            site,
            source: anyhow!("unexpected status code: {}", status_code.as_u16()),
        });
    }

    let response_bytes = match response.bytes().await {
        Ok(data) => data,
        Err(e) => {
            return Err(CheckError {
                site,
                source: anyhow!(e).context("failed to parse response"),
            })
        }
    };

    let response_hash = Sha3_224::digest(&response_bytes);
    let response_hash_string = HexFormat(response_hash.as_ref()).to_string();
    let updated = match &site.hash {
        Some(data) => data != &response_hash_string,
        None => true,
    };

    Ok(CheckOk {
        updated,
        site: Site {
            hash: Some(response_hash_string),
            ..site
        },
    })
}

fn generate_slack_payload(
    bot_name: &str,
    channel_id: &str,
    updated_sites: &[Rc<Site>],
    error_sites: &[(Rc<Site>, anyhow::Error)],
) -> Fallible<String> {
    use serde_json::json;

    let mut blocks = vec![];
    let mut text_source = vec![];

    if !updated_sites.is_empty() {
        blocks.push(json!({
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": "Updated",
            },
        }));

        for site in updated_sites {
            blocks.push(json!({
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": format!(
                        "<{}|{}>",
                        site.uri_open.as_ref().unwrap_or(&site.uri),
                        site.title,
                    ),
                },
            }));
        }

        text_source.push(format!("{} updates", updated_sites.len()));
    }

    if !updated_sites.is_empty() && !error_sites.is_empty() {
        blocks.push(json!({ "type": "divider" }));
        text_source.push(", ".into());
    }

    if !error_sites.is_empty() {
        blocks.push(json!({
            "type": "header",
            "text": {
                "type": "plain_text",
                "text": "Error",
            },
        }));

        for (site, e) in error_sites {
            blocks.push(json!({
                "type": "section",
                "text": {
                    "type": "mrkdwn",
                    "text": format!(
                        "<{}|{}>\n>{}",
                        site.uri_open.as_ref().unwrap_or(&site.uri),
                        site.title,
                        e,
                    ),
                },
            }));
        }

        text_source.push(format!("{} errors:", error_sites.len()));
    }

    let payload = json!({
        "channel": channel_id,
        "icon_emoji": ":new:",
        "username": bot_name,
        "blocks": blocks,
        "text": text_source.join(""),
    });
    Ok(serde_json::to_string(&payload)?)
}
