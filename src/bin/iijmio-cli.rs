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

use axum::extract::Query;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Router, Server};
use chrono::{DateTime, Duration, Utc};
use rand::distributions::{Alphanumeric, DistString};
use rand::thread_rng;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use reqwest::Client;
use rust_myscript::prelude::*;
use serde::de::{self, Unexpected, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt::{Debug, Formatter};
use std::fs::{create_dir_all, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{Ipv4Addr, SocketAddr};
use tokio::signal;
use tokio::sync::broadcast::{channel, Receiver, Sender};
use url::Url;

#[tokio::main]
async fn main() -> Fallible<()> {
    let client_id = match option_env!("IIJMIO_CLI_CLIENT_ID") {
        Some(data) => data,
        None => {
            bail!("need client id");
        }
    };

    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("Hello");

    let project_dirs = directories::ProjectDirs::from("com", "sukawasatoru", "IIJmio cli")
        .context("no valid home directory")?;

    let current_time = Utc::now();

    let mut prefs = load_prefs(project_dirs.config_dir())?;
    let access_token = match prefs.access_token {
        Some(access_token) => {
            let a = prefs.access_token_expires - current_time;
            if a < Duration::minutes(1) {
                let (access_token, expires_in) = authn(client_id).await?;
                prefs.access_token = Some(access_token.clone());
                prefs.access_token_expires = current_time + expires_in;
                store_prefs(project_dirs.config_dir(), &prefs)?;
                access_token
            } else {
                access_token
            }
        }
        None => {
            let (access_token, expires_in) = authn(client_id).await?;
            prefs.access_token = Some(access_token.clone());
            prefs.access_token_expires = current_time + expires_in;
            store_prefs(project_dirs.config_dir(), &prefs)?;

            access_token
        }
    };

    let api_base_uri = Url::parse("https://api.iijmio.jp")?;
    let _coupon_api_endpoint = api_base_uri.join("mobile/d/v2/coupon")?;
    let packet_api_endpoint = api_base_uri.join("mobile/d/v2/log/packet")?;
    let client = create_client(client_id, &access_token)?;

    let res = client
        .get(packet_api_endpoint.as_str())
        .send()
        .await?
        .error_for_status()?
        .json::<serde_json::Value>()
        .await?;

    println!("{}", serde_json::to_string_pretty(&res)?);

    info!("Bye");

    Ok(())
}

async fn authn(client_id: &str) -> Fallible<(String, Duration)> {
    let redirect_uri = Url::parse("http://127.0.0.1:38088/callback")?;

    // https://www.iijmio.jp/hdd/coupon/mioponapi.html
    let authn_state = create_authn_state();
    let mut authn_endpoint = Url::parse("https://api.iijmio.jp/mobile/d/v1/authorization/")?;
    authn_endpoint.query_pairs_mut().extend_pairs(&[
        ("response_type", "token"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri.as_str()),
        ("state", &authn_state),
    ]);

    opener::open_browser(authn_endpoint.as_str())?;

    launch_callback_server(authn_state, redirect_uri.port().expect("port")).await
}

async fn launch_callback_server(
    authn_state: String,
    server_port: u16,
) -> Fallible<(String, Duration)> {
    let (tx, mut rx) = channel(1);
    let app = Router::new()
        .route("/callback", get(handler_callback))
        .route(
            "/callback2",
            get({
                let tx = tx.clone();
                move |params: Query<Callback2Params>| handler_callback2(params, tx, authn_state)
            }),
        );

    Server::bind(&SocketAddr::from((Ipv4Addr::LOCALHOST, server_port)))
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal(tx.subscribe()))
        .await
        .context("server error")?;

    match rx.try_recv() {
        Ok(data) => match data {
            Ok(data) => {
                debug!(?data, "succeeded");
                Ok(data)
            }
            Err(e) => bail!("{}", e),
        },
        Err(e) => {
            debug!(?e);
            bail!("cancelled");
        }
    }
}

/// Redirect to `/callback2` for retrieving the URL fragment.
async fn handler_callback() -> Response {
    Html(
        r#"
<!DOCTYPE html>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width">
<title>redirect</title>
<script>
  "use strict";
  location.href = `/callback2?ref=${encodeURIComponent(location.href)}`;
</script>
<p>
  Please wait a moment.
</p>
"#
        .trim(),
    )
    .into_response()
}

/// Parse [handler_callback] parameters.
#[tracing::instrument]
async fn handler_callback2(
    Query(params): Query<Callback2Params>,
    tx: Sender<Result<(String, Duration), String>>,
    authn_state: String,
) -> Response {
    fn create_response_html(description: &str) -> Html<String> {
        Html(format!(
            r#"
<!DOCTYPE html>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width">
<title>OAuth result</title>
<p>
  {description}
</p>
"#,
        ))
    }

    let params = match params.referer.fragment() {
        Some(data) => data,
        None => return create_response_html("Wrong parameter.").into_response(),
    };

    let (token, expires_in, state) = match deserialize_authn_response(params) {
        Ok(FragmentParams::Success {
            access_token,
            expires_in,
            state,
            ..
        }) => (access_token, expires_in, state),
        Ok(FragmentParams::Error {
            error,
            error_description,
            state,
        }) => {
            return if authn_state == state {
                tx.send(Err(format!(
                    "error: {error:?}, description: {error_description}"
                )))
                .expect("all receivers dropped");
                create_response_html(&error_description).into_response()
            } else {
                info!("received illegal authentication state");
                create_response_html("received illegal authentication state").into_response()
            }
        }
        Err(e) => {
            info!(?e);
            return create_response_html("Wrong parameter.").into_response();
        }
    };

    if authn_state != state {
        return create_response_html("received illegal authentication state").into_response();
    }

    tx.send(Ok((token, expires_in)))
        .expect("all receivers dropped");
    create_response_html("Succeeded. Please close this page.").into_response()
}

async fn shutdown_signal(mut rx: Receiver<Result<(String, Duration), String>>) {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    let accessed = async move { rx.recv().await };

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
        _ = accessed => {},
    }
}

fn create_client(client_id: &str, access_token: &str) -> Fallible<Client> {
    let headers = HeaderMap::from_iter([
        (
            header::ACCEPT,
            HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
        ),
        (
            header::USER_AGENT,
            HeaderValue::from_static("IIJmio cli (https://github.com/sukawasatoru/rust-myscript)"),
        ),
        (
            HeaderName::from_lowercase(b"x-iijmio-developer")?,
            client_id.parse()?,
        ),
        (
            HeaderName::from_lowercase(b"x-iijmio-authorization")?,
            access_token.parse()?,
        ),
    ]);
    Ok(Client::builder().default_headers(headers).build()?)
}

/// Deserialize callback parameter for [handler_callback2].
///
/// The serde's `untagged` attribute has deserialize problem. This function deserialize response
/// avoiding `untagged` problem.
///
/// ref. [nox/serde_urlencoded#33](https://github.com/nox/serde_urlencoded/issues/33)
fn deserialize_authn_response(value: &str) -> Result<FragmentParams, de::value::Error> {
    #[derive(Deserialize)]
    struct S {
        access_token: String,
        token_type: AuthnTokenType,
        #[serde(deserialize_with = "deserialize_expires_in")]
        expires_in: Duration,
        state: String,
    }

    impl From<S> for FragmentParams {
        fn from(value: S) -> Self {
            Self::Success {
                access_token: value.access_token,
                token_type: value.token_type,
                expires_in: value.expires_in,
                state: value.state,
            }
        }
    }

    #[derive(Deserialize)]
    struct E {
        error: AuthnErrorReason,
        error_description: String,
        state: String,
    }

    impl From<E> for FragmentParams {
        fn from(value: E) -> Self {
            Self::Error {
                error: value.error,
                error_description: value.error_description,
                state: value.state,
            }
        }
    }

    let ret = serde_urlencoded::from_str::<S>(value);
    if let Ok(data) = ret {
        return Ok(data.into());
    };

    match serde_urlencoded::from_str::<E>(value) {
        Ok(data) => Ok(data.into()),
        Err(e) => Err(e),
    }
}

#[derive(Deserialize)]
struct Callback2Params {
    #[serde(rename = "ref")]
    referer: Url,
}

impl Debug for Callback2Params {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.referer.as_str())
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
enum FragmentParams {
    Success {
        access_token: String,
        #[allow(unused)]
        token_type: AuthnTokenType,
        #[serde(deserialize_with = "deserialize_expires_in")]
        expires_in: Duration,
        state: String,
    },
    Error {
        error: AuthnErrorReason,
        error_description: String,
        state: String,
    },
}

fn create_authn_state() -> String {
    Alphanumeric.sample_string(&mut thread_rng(), 4)
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
enum AuthnTokenType {
    Bearer,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum AuthnErrorReason {
    InvalidRequest,
    UnsupportedResponseType,
    ServerError,
}

fn deserialize_expires_in<'de, D>(de: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    struct NumberVisitor;
    impl<'de> Visitor<'de> for NumberVisitor {
        type Value = i64;

        fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
            formatter.write_str("an i64")
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            debug!("NumberVisitor.visit_i64");
            Ok(v)
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            debug!("NumberVisitor.visit_u64");
            match i64::try_from(v) {
                Ok(data) => self.visit_i64(data),
                Err(e) => {
                    debug!(?e);
                    Err(E::invalid_type(Unexpected::Unsigned(v), &self))
                }
            }
        }
    }

    Ok(Duration::seconds(de.deserialize_i64(NumberVisitor)?))
}

#[derive(Default, Deserialize, Serialize)]
struct Prefs {
    access_token: Option<String>,
    access_token_expires: DateTime<Utc>,
}

const PREFS_NAME: &str = "preferences.toml";

fn load_prefs(config_path: &std::path::Path) -> Fallible<Prefs> {
    if !config_path.exists() {
        create_dir_all(config_path)?;
    }

    let file_path = config_path.join(PREFS_NAME);

    if !file_path.exists() {
        let prefs = Prefs::default();
        let mut buf = BufWriter::new(File::create(&file_path)?);
        buf.write_all(toml::to_string(&prefs)?.as_bytes())?;
        buf.flush()?;
        return Ok(prefs);
    }

    let mut buf = BufReader::new(File::open(&file_path)?);
    let mut prefs_string = String::new();
    buf.read_to_string(&mut prefs_string)?;
    Ok(toml::from_str(&prefs_string)?)
}

fn store_prefs(config_path: &std::path::Path, prefs: &Prefs) -> Fallible<()> {
    if !config_path.exists() {
        create_dir_all(config_path)?;
    }

    let file_path = config_path.join(PREFS_NAME);

    let mut buf = BufWriter::new(File::create(file_path)?);
    buf.write_all(toml::to_string(&prefs)?.as_bytes())?;
    buf.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AuthnErrorReason::InvalidRequest;
    use crate::AuthnTokenType::Bearer;

    #[test]
    fn authn_response() {
        let cb_url = Url::parse("http://127.0.0.1:38088/callback#access_token=my+access+token&state=my+state&token_type=Bearer&expires_in=3600").unwrap();

        let actual = deserialize_authn_response(cb_url.fragment().unwrap()).unwrap();
        let expected = FragmentParams::Success {
            access_token: "my access token".into(),
            token_type: Bearer,
            expires_in: Duration::hours(1),
            state: "my state".into(),
        };

        assert_eq!(expected, actual);
    }

    #[test]
    fn authn_response_unexpected_expires_in() {
        let cb_url = Url::parse(&format!("http://127.0.0.1:38088/callback#access_token=my+access+token&state=my+state&token_type=Bearer&expires_in={}", (i64::MAX as u64) + 1)).unwrap();

        let actual = deserialize_authn_response(cb_url.fragment().unwrap());
        assert!(actual.is_err());
    }

    #[test]
    fn deserialize_authn_error() {
        let cb_url = Url::parse("http://127.0.0.1:38088/callback#error=invalid_request&error_description=my+error+description&state=my+state").unwrap();
        let actual = deserialize_authn_response(cb_url.fragment().unwrap()).unwrap();
        let expected = FragmentParams::Error {
            error: InvalidRequest,
            error_description: "my error description".into(),
            state: "my state".into(),
        };

        assert_eq!(expected, actual);
    }
}
