use clap::Parser;
use rust_myscript::prelude::*;
use std::net::Ipv4Addr;
use std::sync::Arc;

#[derive(Debug, Parser)]
struct Opt {
    /// Starting address of the sequence
    #[arg(short, long)]
    start_address: Ipv4Addr,

    /// Upper limit
    #[arg(short, long)]
    end_address: Ipv4Addr,

    /// Maximum time in milliseconds
    #[arg(short, long, default_value = "100")]
    timeout: u64,

    /// Maximum number of concurrent http connection
    #[arg(short, long, default_value = "8")]
    parallel_http_connection: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum SystemMode {
    Bridge,
    PPPoERouter,
    LocalRouter,
    WirelessLANClient,
    WirelessLANExtender,
    MapE,
    _464XLAT,
    DsLite,
    FixIP1,
    MultipleFixIP,
    MeshRelay,
    Unknown,
}

impl std::fmt::Display for SystemMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemMode::Bridge => write!(f, "Bridge"),
            SystemMode::PPPoERouter => write!(f, "PPPoE Router"),
            SystemMode::LocalRouter => write!(f, "Local Router"),
            SystemMode::WirelessLANClient => write!(f, "Wireless LAN Client"),
            SystemMode::WirelessLANExtender => write!(f, "Wireless LAN Extender"),
            SystemMode::MapE => write!(f, "MAP-E"),
            SystemMode::_464XLAT => write!(f, "464XLAT"),
            SystemMode::DsLite => write!(f, "DS-Lite"),
            SystemMode::FixIP1 => write!(f, "固定IP1"),
            SystemMode::MultipleFixIP => write!(f, "複数固定IP"),
            SystemMode::MeshRelay => write!(f, "メッシュ中継機"),
            SystemMode::Unknown => write!(f, "-"),
        }
    }
}

impl From<i32> for SystemMode {
    fn from(code: i32) -> Self {
        match code {
            0 => SystemMode::Bridge,
            1 => SystemMode::PPPoERouter,
            2 => SystemMode::LocalRouter,
            3 => SystemMode::WirelessLANClient,
            4 => SystemMode::WirelessLANExtender,
            5 => SystemMode::MapE,
            6 => SystemMode::_464XLAT,
            7 => SystemMode::DsLite,
            8 => SystemMode::FixIP1,
            9 => SystemMode::MultipleFixIP,
            10 => SystemMode::MeshRelay,
            _ => SystemMode::Unknown,
        }
    }
}

fn system_mode_from_body(regex: &regex::Regex, response: &str) -> SystemMode {
    regex
        .captures(response.trim())
        .and_then(|caps| caps.get(1))
        .and_then(|matched| matched.as_str().parse::<i32>().ok())
        .map(SystemMode::from)
        .unwrap_or(SystemMode::Unknown)
}

struct Context {
    regex_product_name: regex::Regex,
    regex_system_mode: regex::Regex,
    timeout: std::time::Duration,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("hello!");

    if std::env::args().any(|data| data == "--debug-server") {
        return debug_server::run().await;
    }

    let opt: Opt = Opt::parse();

    let start_oct = opt.start_address.octets();
    let end_oct = opt.end_address.octets();

    if start_oct[0..=2] != end_oct[0..=2] {
        eprintln!(
            "too large range. please set {}.{}.{}.n to '--end-address'.",
            start_oct[0], start_oct[1], start_oct[2]
        );
        std::process::exit(1);
    }

    let context = Arc::new(Context {
        regex_product_name: regex::Regex::new(r"^PRODUCT_NAME=(.*)$")?,
        regex_system_mode: regex::Regex::new(r"^SYSTEM_MODE=(\d*)$")?,
        timeout: std::time::Duration::from_millis(opt.timeout),
    });

    let client = reqwest::Client::new();

    let results = parallel_strategy(
        context,
        client,
        &opt.start_address,
        &opt.end_address,
        opt.parallel_http_connection,
    )
    .await?;

    println!("results:");
    for (ip_address, product_name, system_mode) in results {
        println!("address: {ip_address}, product name: {product_name}, system mode: {system_mode}");
    }

    info!("bye");

    Ok(())
}

async fn parallel_strategy(
    context: Arc<Context>,
    client: reqwest::Client,
    start_address: &Ipv4Addr,
    end_address: &Ipv4Addr,
    parallel_connection: usize,
) -> anyhow::Result<Vec<(Ipv4Addr, String, SystemMode)>> {
    let mut current_oct = start_address.octets();
    let end_oct = end_address.octets();
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(parallel_connection));

    loop {
        let address = current_oct.into();
        let context = context.clone();
        let client = client.clone();
        let tx = tx.clone();
        let semaphore = semaphore.clone();
        tokio::task::spawn(async move {
            let _permit = semaphore.acquire().await;
            let context = context;
            let product_name =
                match retrieve_product_name(context.clone(), client.clone(), &address).await {
                    Ok(data) => data,
                    Err(e) => {
                        trace!(err = ?e);
                        eprint!(".");
                        return;
                    }
                };

            let system_mode =
                match retrieve_system_mode(context.clone(), client.clone(), &current_oct.into())
                    .await
                {
                    Ok(data) => data,
                    Err(e) => {
                        trace!(err = ?e);
                        eprint!(".");
                        return;
                    }
                };

            if tx
                .send((Ipv4Addr::from(current_oct), product_name, system_mode))
                .await
                .is_err()
            {
                warn!("failed to send result");
                return;
            }
            eprint!("!");
        });

        if current_oct == end_oct {
            break;
        }

        current_oct[3] += 1;
    }

    // drop unused original tx.
    drop(tx);

    let mut ret = vec![];
    while let Some(data) = rx.recv().await {
        ret.push(data);
    }

    eprintln!();

    Ok(ret)
}

async fn retrieve_product_name(
    context: Arc<Context>,
    client: reqwest::Client,
    target: &Ipv4Addr,
) -> anyhow::Result<String> {
    let mut form_data = std::collections::HashMap::new();
    form_data.insert("REQ_ID", "PRODUCT_NAME_GET");

    let result_string = request_aterm(client, target, &context.timeout, &form_data).await?;
    debug!(ip = %target, %result_string);
    let product_name = context
        .regex_product_name
        .captures(result_string.trim())
        .context("captures")?
        .get(1)
        .context("captures.get(1)")?
        .as_str();

    Ok(product_name.into())
}

async fn retrieve_system_mode(
    context: Arc<Context>,
    client: reqwest::Client,
    target: &Ipv4Addr,
) -> anyhow::Result<SystemMode> {
    let mut form_data = std::collections::HashMap::new();
    form_data.insert("REQ_ID", "SYS_MODE_GET");

    let response_string = request_aterm(client, target, &context.timeout, &form_data).await?;
    trace!(ip = %target, %response_string);

    Ok(system_mode_from_body(
        &context.regex_system_mode,
        &response_string,
    ))
}

async fn request_aterm(
    client: reqwest::Client,
    target: &Ipv4Addr,
    timeout: &std::time::Duration,
    form_data: &std::collections::HashMap<&'static str, &'static str>,
) -> anyhow::Result<String> {
    trace!(ip = %target, from = ?form_data, "request");
    let response = client
        .post(format!(
            "http://{target}/aterm_httpif.cgi/getparamcmd_no_auth"
        ))
        .form(&form_data)
        .timeout(*timeout)
        .send()
        .await?;
    trace!(ip = %target, body = ?response, "response");

    match response.error_for_status() {
        Ok(ret) => Ok(ret.text().await?),
        Err(e) => {
            debug!(err = ?e);
            Err(e.into())
        }
    }
}

mod debug_server {
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Form, Router};
    use serde::Deserialize;
    use std::net::SocketAddr;

    #[derive(Debug, Deserialize)]
    enum RequestId {
        #[serde(rename = "PRODUCT_NAME_GET")]
        ProductNameGet,

        #[serde(rename = "SYS_MODE_GET")]
        SysModeGet,
    }

    #[derive(Deserialize)]
    struct RequestPayload {
        #[serde(rename = "REQ_ID")]
        req_id: RequestId,
    }

    async fn handle_request(Form(payload): Form<RequestPayload>) -> impl IntoResponse {
        let body = match payload.req_id {
            RequestId::ProductNameGet => "PRODUCT_NAME=WG1200HS2\r\n",
            RequestId::SysModeGet => "SYSTEM_MODE=0\r\n",
        };
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/html"),
                (header::SERVER, "Aterm(CR)/1.0.0"),
                (header::PRAGMA, "no-cache"),
                (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"),
                (header::EXPIRES, "0"),
            ],
            body,
        )
    }

    pub async fn run() -> anyhow::Result<()> {
        let app = Router::new().route(
            "/aterm_httpif.cgi/getparamcmd_no_auth",
            post(handle_request),
        );
        let addr = SocketAddr::from(([0, 0, 0, 0], 80));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_product_name_should_capture_product_name() {
        let reg = regex::Regex::new(r"^PRODUCT_NAME=(.*)$").unwrap();
        let cap = reg.captures(r"PRODUCT_NAME=aterm").unwrap();
        assert_eq!("aterm", cap.get(1).unwrap().as_str())
    }

    #[test]
    fn regex_system_mode_should_capture_system_mode() {
        let reg = regex::Regex::new(r"^SYSTEM_MODE=(\d*)$").unwrap();
        let actual = reg
            .captures(r"SYSTEM_MODE=2")
            .unwrap()
            .get(1)
            .unwrap()
            .as_str()
            .parse::<i32>()
            .unwrap();
        assert_eq!(2, actual)
    }

    #[test]
    fn system_mode_from_body_should_parse_supported_modes() {
        let regex = regex::Regex::new(r"^SYSTEM_MODE=(\d*)$").unwrap();
        let test_cases = [
            ("SYSTEM_MODE=0", SystemMode::Bridge),
            ("SYSTEM_MODE=1", SystemMode::PPPoERouter),
            ("SYSTEM_MODE=2", SystemMode::LocalRouter),
            ("SYSTEM_MODE=3", SystemMode::WirelessLANClient),
            ("SYSTEM_MODE=4", SystemMode::WirelessLANExtender),
            ("SYSTEM_MODE=5", SystemMode::MapE),
            ("SYSTEM_MODE=6", SystemMode::_464XLAT),
            ("SYSTEM_MODE=7", SystemMode::DsLite),
            ("SYSTEM_MODE=8", SystemMode::FixIP1),
            ("SYSTEM_MODE=9", SystemMode::MultipleFixIP),
            ("SYSTEM_MODE=10", SystemMode::MeshRelay),
        ];

        for (body, expected) in test_cases {
            assert_eq!(expected, system_mode_from_body(&regex, body));
        }
    }

    #[test]
    fn system_mode_from_body_should_return_unknown_for_unsupported_modes() {
        let regex = regex::Regex::new(r"^SYSTEM_MODE=(\d*)$").unwrap();

        for body in ["SYSTEM_MODE=11", "SYSTEM_MODE=999"] {
            assert_eq!(SystemMode::Unknown, system_mode_from_body(&regex, body));
        }
    }

    #[test]
    fn system_mode_from_body_should_return_unknown_for_invalid_bodies() {
        let regex = regex::Regex::new(r"^SYSTEM_MODE=(\d*)$").unwrap();

        for body in ["", "SYSTEM_MODE=", "SYSTEM_MODE=-1", "invalid"] {
            assert_eq!(SystemMode::Unknown, system_mode_from_body(&regex, body));
        }
    }

    #[test]
    fn system_mode_from_body_should_ignore_trailing_whitespace() {
        let regex = regex::Regex::new(r"^SYSTEM_MODE=(\d*)$").unwrap();

        assert_eq!(
            SystemMode::MeshRelay,
            system_mode_from_body(&regex, "SYSTEM_MODE=10\r\n")
        );
    }
}
