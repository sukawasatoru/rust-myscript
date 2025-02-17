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

use chrono::Datelike;
use clap::builder::ArgPredicate;
use clap::{Args, Parser, ValueEnum};
use reqwest::header;
use rust_myscript::prelude::*;
use serde_json::json;
use std::fmt::{Display, Formatter, Write};
use std::time::Duration;

#[derive(Parser)]
struct Opt {
    /// Model name.
    #[arg(short, long)]
    model: OptModel,

    /// API Key for Perplexity AI
    #[arg(long, env)]
    api_key: String,

    #[command(flatten)]
    telegram: Option<OptTelegram>,
}

#[derive(Clone, ValueEnum)]
enum OptModel {
    SonarPro,
    SonarReasoningPro,
}

impl Display for OptModel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            OptModel::SonarPro => f.write_str("sonar-pro"),
            OptModel::SonarReasoningPro => f.write_str("sonar-reasoning-pro"),
        }
    }
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

    /// Template to send message that include `{pplx}` to insert value
    #[arg(long, env, requires = "use_telegram")]
    telegram_text_template: Option<String>,
}

fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let opt = Opt::parse();

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60 * 5))
        .user_agent("pplx-news (https://github.com/sukawasatoru/rust-myscript/)")
        .build()?;

    let current = chrono::Local::now();
    let res = client
        .post("https://api.perplexity.ai/chat/completions")
        .header(header::ACCEPT, "application/json")
        .header(header::CONTENT_TYPE, "application/json")
        .bearer_auth(&opt.api_key)
        .body(serde_json::to_string(&json!({
            "model": opt.model.to_string(),
            "search_recency_filter": "day",
            "messages": [
                {
                    "role": "user",
                    "content": format!(
                        "{}-{:02}-{:02} のニュース",
                        current.year(),
                        current.month(),
                        current.day(),
                    ),
                }
            ]
        }))?)
        .send()?;

    info!(?res);
    let res_text = res.text()?;
    debug!(%res_text);

    let res = serde_json::from_str::<serde_json::Value>(&res_text)?;
    let (pplx_content, pplx_citations) = deconstruct_payload(&res)?;

    println!(
        "{pplx_content}\n{}",
        pplx_citations
            .iter()
            .enumerate()
            .map(|(i, data)| format!("[{}] {}", i + 1, data))
            .collect::<Vec<_>>()
            .join("\n")
    );

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
                pplx_content,
                &pplx_citations,
            )?)
            .send()?;
        info!(?ret_telegram);
        debug!(ret_telegram_text = %ret_telegram.text()?);
    }

    Ok(())
}

fn deconstruct_payload(value: &serde_json::Value) -> Fallible<(&str, Vec<&str>)> {
    Ok((
        value["choices"][0]["message"]["content"]
            .as_str()
            .context("choices[0].message.content should be string")?,
        value["citations"]
            .as_array()
            .context("citations")?
            .iter()
            .map(|data| data.as_str().expect("citations should be string"))
            .collect::<Vec<_>>(),
    ))
}

fn generate_telegram_payload(
    chat_id: &str,
    template_txt: &str,
    pplx_content: &str,
    pplx_citations: &Vec<&str>,
) -> Fallible<String> {
    let reg = regex::Regex::new(r#"([_\[\]()~`>#+=\-|{}.!])"#)?;

    // content 1. replace pplx strong to telegram's strong.
    // content 2. escape for telegram's markdownv2.
    // content 3. replace escaped refer mark to link.
    let mut pplx_content = reg
        .replace_all(&pplx_content.replace("**", "*"), r#"\$1"#)
        .into_owned();
    for (i, &entry) in pplx_citations.iter().enumerate() {
        let index = i + 1;
        pplx_content = pplx_content.replace(
            &format!(r"\[{index}\]"),
            &format!(r"[\[{index}\]]({entry})"),
        );
    }

    let pplx_content_w_citations = format!(
        "{}\n\\- \\- \\-\n{}",
        pplx_content,
        pplx_citations
            .iter()
            .enumerate()
            .map(|(i, data)| {
                let mut line = String::with_capacity(data.len() + 4);
                match i {
                    0 => line.write_str("**>[")?,
                    _ => line.write_str(">[")?,
                };
                line.write_fmt(format_args!(
                    "{}] {}",
                    i + 1,
                    reg.replace_all(&data.chars().take(36).collect::<String>(), r#"\$1"#),
                ))?;

                if i == pplx_citations.len() - 1 {
                    line.write_str("||")?;
                }
                Ok(line)
            })
            .collect::<Fallible<Vec<String>>>()?
            .join("\n")
    );

    // template 1. replace `\n` string to new line for serde_json::json.
    // template 2. escape template for telegram's markdownv2.
    // template 3. embedding escaped text(w/ escaped citations) in an escaped template.
    let text = template_txt.replace(r#"\n"#, "\n");
    let text = reg
        .replace_all(&text, r#"\$1"#)
        .replace(r"\{pplx\}", &pplx_content_w_citations);

    debug!(%text);

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
            "pplx-news",
            "--model",
            "sonar-pro",
            "--api-key",
            "api-key",
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
            "pplx-news",
            "--model",
            "sonar-pro",
            "--api-key",
            "api-key",
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
            "pplx-news",
            "--model",
            "sonar-pro",
            "--api-key",
            "api-key",
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
            "pplx-news",
            "--model",
            "sonar-pro",
            "--api-key",
            "api-key",
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
            "pplx-news",
            "--model",
            "sonar-pro",
            "--api-key",
            "api-key",
            "--use-telegram",
            "--telegram-bot-token",
            "token",
            "--telegram-chat-id",
            "chat-id",
        ]);
        assert!(opt.is_err());
    }

    #[test]
    fn generate_telegram_payload_ok() {
        let res = serde_json::from_str::<serde_json::Value>(RES_TEXT).unwrap();
        let (content, citations) = deconstruct_payload(&res).unwrap();
        generate_telegram_payload("123", "template text\n{pplx}", content, &citations).unwrap();
    }

    const RES_TEXT: &str = r#"
{
  "id": "4ecf86c7-f597-46dc-ad1b-784d383bd319",
  "model": "sonar-pro",
  "created": 1234567890,
  "usage": {
    "prompt_tokens": 9,
    "completion_tokens": 525,
    "total_tokens": 534,
    "citation_tokens": 4941,
    "num_search_queries": 2
  },
  "citations": [
    "https://example.com/1",
    "https://example.com/2",
    "https://example.com/3",
    "https://example.com/4",
    "https://example.com/5",
    "https://example.com/6",
    "https://example.com/7",
    "https://example.com/8",
    "https://example.com/9",
    "https://example.com/10"
  ],
  "object": "chat.completion",
  "choices": [
    {
      "index": 0,
      "finish_reason": "stop",
      "message": {
        "role": "assistant",
        "content": "Lorem ipsum dolor sit amet:\n\n1. consectetur adipiscing elit:\nsed do eiusmod tempor incididunt ut labore et dolore magna aliqua[1].\n\n2. Ut enim ad minim veniam:\n- quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat[2].\n- Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur[4].\n- Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum[5].\n\n3. Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium:\n- totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo[4].\n\n4. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugit:\n- sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt[4].\n- Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem[2].\n\n5. Ut enim ad minima veniam:\n- quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur[7]?\n- Quis autem vel eum iure reprehenderit qui in ea voluptate velit esse quam nihil molestiae consequatur, vel illum qui dolorem eum fugiat quo voluptas nulla pariatur[7]?\n\nBut I must explain to you how all this mistaken idea of denouncing pleasure and praising pain was born and I will give you a complete account of the system, and expound the actual teachings of the great explorer of the truth, the master-builder of human happiness."
      },
      "delta": {
        "role": "assistant",
        "content": ""
      }
    }
  ]
}
"#;
}
