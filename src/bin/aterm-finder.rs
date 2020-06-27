use anyhow::Context as AnyhowContext;
use log::{debug, info, trace, warn};
use rust_myscript::myscript::prelude::*;
use std::convert::TryInto;
use std::net::Ipv4Addr;
use std::sync::Arc;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
struct Opt {
    /// Starting address of the sequence
    #[structopt(short, long, parse(try_from_str))]
    start_address: Ipv4Addr,

    /// Upper limit
    #[structopt(short, long, parse(try_from_str))]
    end_address: Ipv4Addr,

    /// Maximum time in milliseconds
    #[structopt(short, long, default_value = "100")]
    timeout: u64,

    /// Maximum number of concurrent http connection
    #[structopt(short, long, default_value = "8")]
    parallel_http_connection: usize,
}

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
        }
    }
}

impl TryInto<SystemMode> for i32 {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<SystemMode, Self::Error> {
        match self {
            0 => Ok(SystemMode::Bridge),
            1 => Ok(SystemMode::PPPoERouter),
            2 => Ok(SystemMode::LocalRouter),
            3 => Ok(SystemMode::WirelessLANClient),
            4 => Ok(SystemMode::WirelessLANExtender),
            5 => Ok(SystemMode::MapE),
            6 => Ok(SystemMode::_464XLAT),
            7 => Ok(SystemMode::DsLite),
            8 => Ok(SystemMode::FixIP1),
            9 => Ok(SystemMode::MultipleFixIP),
            _ => anyhow::bail!("unsupported number: {}", self),
        }
    }
}

struct Context {
    regex_product_name: regex::Regex,
    regex_system_mode: regex::Regex,
    timeout: std::time::Duration,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();

    info!("hello!");

    if std::env::args()
        .find(|data| data == "--debug-server")
        .is_some()
    {
        return debug_server::run().await;
    }

    let opt: Opt = Opt::from_args();

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

    // let results = serial_strategy(context, client, &opt.start_address, &opt.end_address).await?;
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
        println!(
            "address: {}, product name: {}, system mode: {}",
            ip_address, product_name, system_mode
        );
    }

    info!("bye");

    Ok(())
}

#[allow(dead_code)]
async fn serial_strategy(
    context: Arc<Context>,
    client: reqwest::Client,
    start_address: &Ipv4Addr,
    end_address: &Ipv4Addr,
) -> anyhow::Result<Vec<(Ipv4Addr, String, SystemMode)>> {
    let start_oct = start_address.octets();
    let end_oct = end_address.octets();

    let mut current_oct = start_oct;
    let mut results = vec![];

    loop {
        debug!("request: {:?}", current_oct);

        match retrieve_product_name(context.clone(), client.clone(), &current_oct.into()).await {
            Ok(product_name) => {
                match retrieve_system_mode(context.clone(), client.clone(), &current_oct.into())
                    .await
                {
                    Ok(system_mode) => {
                        results.push((Ipv4Addr::from(current_oct), product_name, system_mode));
                        eprint!("!");
                    }
                    Err(e) => {
                        trace!("{:?}", e);
                        eprint!(".");
                    }
                }
            }
            Err(e) => {
                trace!("{:?}", e);
                eprint!(".");
            }
        }

        if current_oct == end_oct {
            break;
        }

        current_oct[3] += 1;
    }

    eprintln!();

    Ok(results)
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
        let mut tx = tx.clone();
        let semaphore = semaphore.clone();
        tokio::task::spawn(async move {
            let _permit = semaphore.acquire().await;
            let context = context;
            let product_name =
                match retrieve_product_name(context.clone(), client.clone(), &address).await {
                    Ok(data) => data,
                    Err(e) => {
                        trace!("{:?}", e);
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
                        trace!("{:?}", e);
                        eprint!(".");
                        return;
                    }
                };

            if let Err(_) = tx
                .send((Ipv4Addr::from(current_oct), product_name, system_mode))
                .await
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
    debug!("ip: {}, result_string: {}", target, result_string);
    let product_name = context
        .regex_product_name
        .captures(result_string.trim())
        .ok_or_err()?
        .get(1)
        .ok_or_err()?
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
    trace!("ip: {}, result: {}", target, response_string);

    let ret = context
        .regex_system_mode
        .captures(response_string.trim())
        .with_context(|| format!("no match found: {}", response_string))?
        .get(1)
        .with_context(|| format!("no match group found: {}", response_string))?
        .as_str()
        .parse::<i32>()?
        .try_into()?;
    Ok(ret)
}

async fn request_aterm(
    client: reqwest::Client,
    target: &Ipv4Addr,
    timeout: &std::time::Duration,
    form_data: &std::collections::HashMap<&'static str, &'static str>,
) -> anyhow::Result<String> {
    trace!("request ip: {}, form: {:?}", target, form_data);
    let response = client
        .post(&format!(
            "http://{}/aterm_httpif.cgi/getparamcmd_no_auth",
            target
        ))
        .form(&form_data)
        .timeout(*timeout)
        .send()
        .await?;
    trace!("response ip: {}, body: {:?}", target, response);

    match response.error_for_status() {
        Ok(ret) => Ok(ret.text().await?),
        Err(e) => {
            debug!("err: {:?}", e);
            Err(e.into())
        }
    }
}

mod debug_server {
    use serde::Deserialize;
    use warp::http::header;
    use warp::Filter;

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

    pub async fn run() -> anyhow::Result<()> {
        let post = warp::post()
            .and(warp::path!("aterm_httpif.cgi" / "getparamcmd_no_auth"))
            .and(warp::body::form())
            .map(|payload: RequestPayload| {
                let body = match payload.req_id {
                    RequestId::ProductNameGet => "PRODUCT_NAME=WG1200HS2\r\n",
                    RequestId::SysModeGet => "SYSTEM_MODE=0\r\n",
                };
                warp::http::Response::builder()
                    .header(header::CONTENT_TYPE, "text/html")
                    .header(header::SERVER, "Aterm(CR)/1.0.0")
                    .header(header::PRAGMA, "no-cache")
                    .header(header::CACHE_CONTROL, "no-store, no-cache, must-revalidate")
                    .header(header::EXPIRES, 0)
                    .body(body)
            });
        warp::serve(post).run(([0, 0, 0, 0], 80)).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_regex_product_name() {
        let reg = regex::Regex::new(r"^PRODUCT_NAME=(.*)$").unwrap();
        let cap = reg.captures(r"PRODUCT_NAME=aterm").unwrap();
        assert_eq!("aterm", cap.get(1).unwrap().as_str())
    }

    #[test]
    fn test_regex_system_mode() {
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
}
