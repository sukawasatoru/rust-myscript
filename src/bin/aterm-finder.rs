use std::convert::TryInto;
use std::net::Ipv4Addr;

use log::debug;
use serde::export::Formatter;
use structopt::StructOpt;

use rust_myscript::myscript::prelude::*;

#[derive(Debug, StructOpt)]
struct Opt {
    #[structopt(short, long, parse(try_from_str))]
    start_address: Ipv4Addr,

    #[structopt(short, long, parse(try_from_str))]
    end_address: Ipv4Addr,
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
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();

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

    let context = Context {
        regex_product_name: regex::Regex::new(r"^PRODUCT_NAME=(.*)$")?,
    };

    let client = reqwest::Client::new();

    let mut current_oct = start_oct;
    let mut results = vec![];

    loop {
        debug!("request: {:?}", current_oct);

        if let Ok(product_name) =
            retrieve_product_name(&context, client.clone(), &current_oct.into()).await
        {
            if let Ok(system_mode) = retrieve_system_mode(client.clone(), &current_oct.into()).await
            {
                results.push((Ipv4Addr::from(current_oct), product_name, system_mode));
                eprint!("!");
            } else {
                eprint!(".");
            }
        } else {
            eprint!(".");
        }

        if current_oct == end_oct {
            break;
        }

        current_oct[3] += 1;
    }

    eprintln!();

    println!("results:");
    for (ip_address, product_name, system_mode) in results {
        println!(
            "address: {}, product name: {}, system mode: {}",
            ip_address, product_name, system_mode
        );
    }

    Ok(())
}

async fn retrieve_product_name(
    context: &Context,
    client: reqwest::Client,
    target: &Ipv4Addr,
) -> anyhow::Result<String> {
    let mut form_data = std::collections::HashMap::new();
    form_data.insert("REQ_ID", "PRODUCT_NAME_GET");

    let result_string = request_aterm(client, target, &form_data).await?;
    let product_name = context
        .regex_product_name
        .captures(&result_string)
        .ok_or_err()?
        .get(1)
        .ok_or_err()?
        .as_str();

    Ok(product_name.into())
}

async fn retrieve_system_mode(
    client: reqwest::Client,
    target: &Ipv4Addr,
) -> anyhow::Result<SystemMode> {
    let mut form_data = std::collections::HashMap::new();
    form_data.insert("REQ_ID", "SYS_MODE_GET");

    let ret = request_aterm(client, target, &form_data)
        .await?
        .parse::<i32>()?
        .try_into()?;
    Ok(ret)
}

async fn request_aterm(
    client: reqwest::Client,
    target: &Ipv4Addr,
    form_data: &std::collections::HashMap<&'static str, &'static str>,
) -> anyhow::Result<String> {
    let response = client
        .post(&format!(
            "http://{}/aterm_httpif.cgi/getparamcmd_no_auth",
            target
        ))
        .form(&form_data)
        .timeout(std::time::Duration::from_millis(100))
        .send()
        .await?;

    match response.error_for_status() {
        Ok(ret) => Ok(ret.text().await?),
        Err(e) => {
            debug!("err: {:?}", e);
            Err(e.into())
        }
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
}
