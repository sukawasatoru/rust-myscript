use clap::Parser;
use regex::Regex;
use rust_myscript::prelude::*;
use std::io::BufRead;
use tracing::{debug, info};

struct Context {
    terminal_notifier_name: String,
    slack_user_name: String,
    slack_notify_url: String,
    reqwest_client: reqwest::Client,
    battery_level_regex: Regex,
    battery_remaining_regex: Regex,
    charging_regex: Regex,
}

struct PSInfo {
    battery_level: u8,
    battery_remaining: String,
    charging: bool,
}

#[derive(Debug, Parser)]
struct Opt {
    /// The threshold of the battery level to notify that between 1 to 99
    #[clap(short = 'l', long, default_value = "40", parse(try_from_str = parse_battery_threshold))]
    battery_level_threshold: u8,

    /// Bot name for slack
    #[clap(long)]
    slack_bot_name: String,

    /// Web hooks URL for slack
    #[clap(long)]
    slack_notify_url: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("Hello");

    let opt: Opt = Opt::parse();

    let notify_threshold = opt.battery_level_threshold;
    let context = Context {
        terminal_notifier_name: "terminal-notifier".to_owned(),
        slack_user_name: opt.slack_bot_name,
        slack_notify_url: opt.slack_notify_url,
        reqwest_client: reqwest::Client::new(),
        battery_level_regex: Regex::new("	([0-9]*)%;")?,
        battery_remaining_regex: Regex::new("; ([0-9:]*) remaining present: true")?,
        charging_regex: Regex::new("; charging;")?,
    };

    let use_terminal_notifier = check_terminal_notifier(&context.terminal_notifier_name);
    debug!(%use_terminal_notifier);

    let process = std::process::Command::new("pmset")
        .args(&["-g", "pslog"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;

    let mut reader = std::io::BufReader::new(process.stdout.context("stdout")?);
    let mut s = String::new();
    let mut previous_level = u8::MAX;
    loop {
        debug!("loop");
        s.clear();
        let read_result = reader.read_line(&mut s)?;
        if read_result == 0 {
            break;
        }

        print!("{}", s);
        let ps_info = match parse_line(&context, &s) {
            Some(data) => data,
            None => continue,
        };
        info!(%ps_info.battery_level, %ps_info.battery_remaining);

        if ps_info.charging {
            continue;
        }

        if ps_info.battery_level <= notify_threshold && ps_info.battery_level < previous_level {
            if use_terminal_notifier {
                notify_terminal(&context, &ps_info).ok();
            }

            match notify_slack(&context, &ps_info).await {
                Ok(data) => info!(%data, "notify success"),
                Err(e) => info!(%e, "notify fail"),
            };
        }

        previous_level = ps_info.battery_level;
    }

    info!("Bye");

    Ok(())
}

fn parse_battery_threshold(value: &str) -> Result<u8, &'static str> {
    match value.parse::<u8>() {
        Ok(data @ 1..=99) => Ok(data),
        _ => Err("expected 1..=99"),
    }
}

fn parse_line(context: &Context, line: &str) -> Option<PSInfo> {
    let battery_level = match context.battery_level_regex.captures(line) {
        Some(data) => match data.get(1).unwrap().as_str().parse() {
            Ok(data) => data,
            Err(_) => return None,
        },
        None => return None,
    };

    let battery_remaining = match context.battery_remaining_regex.captures(line) {
        Some(data) => data.get(1).unwrap().as_str().to_owned(),
        None => return None,
    };

    let charging = context.charging_regex.captures(line).is_some();

    Some(PSInfo {
        battery_level,
        battery_remaining,
        charging,
    })
}

fn check_terminal_notifier(executable_name: &str) -> bool {
    if std::env::var("SSH_TTY").is_ok() {
        debug!("has SSH_TTY");
        return false;
    }

    match std::process::Command::new("command")
        .args(&["-v", executable_name])
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

fn notify_terminal(context: &Context, ps_info: &PSInfo) -> anyhow::Result<()> {
    std::process::Command::new(&context.terminal_notifier_name)
        .args(&vec![
            "-message",
            &format!("Battery: {}", ps_info.battery_level),
        ])
        .status()?;

    Ok(())
}

async fn notify_slack(context: &Context, ps_info: &PSInfo) -> anyhow::Result<String> {
    debug!(payload = %generate_slack_payload(context, ps_info));
    let ret = context
        .reqwest_client
        .post(&context.slack_notify_url)
        .body(generate_slack_payload(context, ps_info))
        .header(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_str("application/x-www-form-urlencoded")?,
        )
        .send()
        .await?
        .text()
        .await?;

    Ok(ret)
}

fn generate_slack_payload(context: &Context, ps_info: &PSInfo) -> String {
    format!(
        r#"payload={{
    "icon_emoji": ":computer:",
    "username": "{}",
    "text": "Battery: {}%, {} remaining present"
}}"#,
        context.slack_user_name, ps_info.battery_level, ps_info.battery_remaining
    )
}
