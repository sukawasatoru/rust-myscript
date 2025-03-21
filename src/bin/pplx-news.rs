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
use regex::Regex;
use reqwest::header;
use rust_myscript::feature::otel::init_otel;
use rust_myscript::prelude::*;
use serde::Serialize;
use serde_json::json;
use std::fmt::{Display, Formatter, Write};
use std::time::Duration;
use tracing_subscriber::EnvFilter;
use url::Url;

#[derive(Parser)]
struct Opt {
    /// Model name.
    #[arg(short, long, default_value = "sonar-pro")]
    model: OptModel,

    /// API Key for Perplexity AI
    #[arg(long, env)]
    api_key: String,

    #[command(flatten)]
    telegram: Option<OptTelegram>,

    /// OpenTelemetry logs endpoint.
    #[arg(long, env)]
    otel_logs_endpoint: Option<Url>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Clone, ValueEnum)]
enum OptModel {
    SonarPro,
    SonarReasoningPro,
    SonarDeepResearch,
}

impl Display for OptModel {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            OptModel::SonarPro => f.write_str("sonar-pro"),
            OptModel::SonarReasoningPro => f.write_str("sonar-reasoning-pro"),
            OptModel::SonarDeepResearch => f.write_str("sonar-deep-research"),
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

#[allow(dead_code)]
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum SearchContextSize {
    High,
    Medium,
    Low,
}

#[allow(dead_code)]
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum SearchRecencyFilter {
    Month,
    Week,
    Day,
    Hour,
}

#[tokio::main]
async fn main() -> Fallible<()> {
    dotenv::dotenv().ok();

    let opt = Opt::parse();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60 * 10))
        .user_agent("pplx-news (https://github.com/sukawasatoru/rust-myscript/)")
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
                .with_env_filter(EnvFilter::from_default_env())
                .with_writer(std::io::stderr)
                .init();
            None
        }
    };

    let current = chrono::Local::now();
    let res = client
        .post("https://api.perplexity.ai/chat/completions")
        .header(header::ACCEPT, "application/json")
        .header(header::CONTENT_TYPE, "application/json")
        .bearer_auth(&opt.api_key)
        .body(serde_json::to_string(&json!({
            "model": opt.model.to_string(),
            // TODO: use `"search_domain_filter": [""]`
            //  https://docs.perplexity.ai/api-reference/chat-completions#body-search-domain-filter
            "web_search_options": {
                "search_context_size": SearchContextSize::High,
            },
            "search_recency_filter": SearchRecencyFilter::Week,
            "messages": [
                {
                    "role": "system",
                    "content": "最新のニュースを提供してください。詳細はユーザーのプロンプトに従ってください。",
                },
                {
                    "role": "user",
                    "content": format!(
                        r#"
{year}年{month}月{day}日の主要ニュースを要約してください。各項目は150字以内で簡潔にまとめ、以下のカテゴリーごとに、信頼性の高い情報源から検証された情報を整理してください：

1. **トップニュース**：過去24時間以内の重要ニュース3点。各ニュースに少なくとも2つの一次情報源と発表時間を明記し、特に公式発表や複数メディアが報じている内容を優先。

2. **テクノロジー・IT**：新製品発表、技術革新、AIやデジタル分野の最新動向3点。企業の公式発表や専門メディアからの情報を引用してください。特に発表から48時間以内の最新情報を優先。

3. **社会・生活**：日本国内の社会現象、健康・安全情報など生活関連ニュース2点。政府機関や公共団体の発表、専門家の見解を含めてください。

4. **今日のトレンド**：Twitter(X)等で拡散している話題1点を、トレンド化の背景や関連データと共に説明してください。

5. **ゲーム**：下記のいずれかに該当するゲーム関連のニュース3点をまとめてください：
- 新作発表・重要アップデート情報（開発元の公式発表があればそれを優先）
- 任天堂関連の公式発表や新作情報
- コンソール/PCゲーム市場の重要な動向
- 人気タイトルのイベント情報
- 業界の経済動向や企業戦略

6. **天気情報**：{year}年{month}月{day}日の全国の気象状況と横浜市の詳細予報。

最後に、ニュース全体から見える重要なトレンドや関連性を3文以内でまとめ、使用した合計文字数を報告してください。
"#,
                        year = current.year(),
                        month = current.month(),
                        day = current.day(),
                    ),
                }
            ]
        }))?)
        .send()
        .await?;

    info!(?res);
    let res_text = res.text().await?;
    debug!(%res_text);

    let res = serde_json::from_str::<serde_json::Value>(&res_text)?;
    let (pplx_content, pplx_citations) = deconstruct_payload(&res)?;
    let pplx_citations = pplx_citations
        .iter()
        .map(|data| {
            Url::parse(data).unwrap_or_else(|e| {
                warn!(?e, url = data, "failed to parse url");
                Url::parse("https://failed-to-parse-url.example.com/").expect("default url")
            })
        })
        .collect::<Vec<_>>();

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
            .send()
            .await?;
        info!(?ret_telegram);
        debug!(ret_telegram_text = %ret_telegram.text().await?);
    }

    if otel_guard.is_some() {
        let current_string = current.to_rfc3339();
        for entry in pplx_citations {
            info!(
                event.name = "device.app.citations",
                datetime = current_string,
                model_name = %opt.model,
                citation_url_domain = %entry.domain().expect("url.domain"),
                citation_url_full = entry.as_str(),
            );
        }
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
    pplx_citations: &[Url],
) -> Fallible<String> {
    let reg = Regex::new(r#"([_\[\]()~`>#+=\-|{}.!])"#)?;

    let reg_remove_think = Regex::new(r"^<think>[\s\S]+</think>(.*)")?;

    // content 1. remove thinking process.
    // content 2. replace pplx strong to telegram's strong.
    // content 3. escape for telegram's markdownv2.
    // content 4. replace escaped refer mark to link.
    let pplx_content = reg_remove_think.replace_all(pplx_content, r"$1");
    let mut pplx_content = reg
        .replace_all(&pplx_content.replace("**", "*"), r#"\$1"#)
        .into_owned();
    for (i, entry) in pplx_citations.iter().enumerate() {
        let index = i + 1;
        pplx_content = pplx_content.replace(
            &format!(r"\[{index}\]"),
            &format!(r"[\[{index}\]]({entry})"),
        );
    }

    // add citations at bottom.
    let pplx_content_w_citations = format!(
        "{}\n\\- \\- \\-\n{}",
        pplx_content,
        pplx_citations
            .iter()
            .enumerate()
            .map(|(i, data)| {
                let mut line = String::new();
                match i {
                    0 => line.write_str(r"**>\[")?,
                    _ => line.write_str(r">\[")?,
                };

                write!(
                    line,
                    r"{}\] [{}]({})",
                    i + 1,
                    reg.replace_all(
                        data.domain().expect("citation should have a domain"),
                        r#"\$1"#
                    ),
                    data,
                )?;

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
        let citations = citations
            .iter()
            .map(|data| Url::parse(data).expect("malformed url"))
            .collect::<Vec<_>>();
        let payload =
            generate_telegram_payload("123", "template text\n{pplx}", content, &citations).unwrap();

        let mut actual_payload_wo_text =
            serde_json::from_str::<serde_json::Value>(&payload).unwrap();
        actual_payload_wo_text
            .as_object_mut()
            .unwrap()
            .remove("text");
        assert_eq!(
            actual_payload_wo_text,
            json!({ "chat_id": "123", "parse_mode": "MarkdownV2" })
        );

        let actual_text = serde_json::from_str::<serde_json::Value>(&payload).unwrap();
        let actual_text = actual_text["text"].as_str().unwrap();
        let expected_text = r#"template text
Lorem ipsum dolor sit amet:

1\. consectetur adipiscing elit:
sed do eiusmod tempor incididunt ut labore et dolore magna aliqua[\[1\]](https://example.com/)\.

2\. Ut enim ad minim veniam:
\- quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat[\[2\]](https://2.example.com/2?foo=bar#baz)\.
\- Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur[\[4\]](https://4.example.com/4)\.
\- Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum[\[5\]](https://5.example.com/5)\.

3\. Sed ut perspiciatis unde omnis iste natus error sit voluptatem accusantium doloremque laudantium:
\- totam rem aperiam, eaque ipsa quae ab illo inventore veritatis et quasi architecto beatae vitae dicta sunt explicabo[\[4\]](https://4.example.com/4)\.

4\. Nemo enim ipsam voluptatem quia voluptas sit aspernatur aut odit aut fugit:
\- sed quia consequuntur magni dolores eos qui ratione voluptatem sequi nesciunt[\[4\]](https://4.example.com/4)\.
\- Neque porro quisquam est, qui dolorem ipsum quia dolor sit amet, consectetur, adipisci velit, sed quia non numquam eius modi tempora incidunt ut labore et dolore magnam aliquam quaerat voluptatem[\[2\]](https://2.example.com/2?foo=bar#baz)\.

5\. Ut enim ad minima veniam:
\- quis nostrum exercitationem ullam corporis suscipit laboriosam, nisi ut aliquid ex ea commodi consequatur[\[7\]](https://7.example.com/7)?
\- Quis autem vel eum iure reprehenderit qui in ea voluptate velit esse quam nihil molestiae consequatur, vel illum qui dolorem eum fugiat quo voluptas nulla pariatur[\[7\]](https://7.example.com/7)?

But I must explain to you how all this mistaken idea of denouncing pleasure and praising pain was born and I will give you a complete account of the system, and expound the actual teachings of the great explorer of the truth, the master\-builder of human happiness\.
\- \- \-
**>\[1\] [example\.com](https://example.com/)
>\[2\] [2\.example\.com](https://2.example.com/2?foo=bar#baz)
>\[3\] [3\.example\.com](https://3.example.com/3)
>\[4\] [4\.example\.com](https://4.example.com/4)
>\[5\] [5\.example\.com](https://5.example.com/5)
>\[6\] [6\.example\.com](https://6.example.com/6)
>\[7\] [7\.example\.com](https://7.example.com/7)
>\[8\] [8\.example\.com](https://8.example.com/8)
>\[9\] [9\.example\.com](https://9.example.com/9)
>\[10\] [10\.example\.com](https://10.example.com/10)||"#;

        assert_eq!(actual_text, expected_text);
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
    "https://example.com",
    "https://2.example.com/2?foo=bar#baz",
    "https://3.example.com/3",
    "https://4.example.com/4",
    "https://5.example.com/5",
    "https://6.example.com/6",
    "https://7.example.com/7",
    "https://8.example.com/8",
    "https://9.example.com/9",
    "https://10.example.com/10"
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
