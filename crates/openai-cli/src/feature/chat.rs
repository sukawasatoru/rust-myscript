/*
 * Copyright 2023, 2024, 2025 sukawasatoru
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

use crate::data::repository::{ChatRepository, GetChatRepository, GetPreferencesRepository};
use crate::feature::chat::select_conversation::{SelectedType, select_conversation};
use crate::feature::chat::string_value_serializer::get_serialized_string;
use crate::functions::{prepare_headers, print_stdin_help};
use crate::model::{Chat, ChatID, Message, MessageID, MessageRole};
use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use ratatui::crossterm::execute;
use ratatui::crossterm::style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor};
use ratatui::crossterm::tty::IsTty;
use reqwest::blocking::Client;
use rust_myscript::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::prelude::*;
use std::io::{BufReader, Read, stdin, stdout};
use std::str::FromStr;
use tracing::instrument;
use uuid::Uuid;

mod select_conversation;
mod string_value_serializer;

pub fn chat<Ctx>(
    context: Ctx,
    arg_organization_id: Option<String>,
    arg_api_key: Option<String>,
    disable_color: bool,
    disable_stream: bool,
    model: Option<String>,
) -> Fallible<()>
where
    Ctx: GetPreferencesRepository,
    Ctx: GetChatRepository,
{
    let disable_color = disable_color || !stdin().is_tty();
    let model = match model {
        Some(data) => data.parse::<ChatCompletionModel>()?,
        None => Default::default(),
    };

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(60 * 5))
        .default_headers(prepare_headers(&context, arg_organization_id, arg_api_key)?)
        .build()?;

    let chat_repo = context.get_chat_repo();
    let mut chat_histories = chat_repo.find_chat_all()?;
    chat_histories.sort_by_key(|(chat, messages)| {
        messages
            .last()
            .map(|data| data.updated_at)
            .unwrap_or(chat.created_at)
    });
    chat_histories.reverse();
    let (mut chat, mut messages) = match select_conversation(&chat_histories)? {
        SelectedType::New => (
            Chat {
                chat_id: ChatID(Uuid::new_v4()),
                title: "".into(),
                created_at: Utc::now(),
                model_id: get_serialized_string(&model)?,
            },
            Vec::<ChatCompletionMessage>::new(),
        ),
        SelectedType::History(selected) => {
            let (chat, messages) = chat_histories
                .into_iter()
                .find(|(chat, _)| chat.chat_id == selected)
                .expect("not found");
            let messages = messages
                .into_iter()
                .map(|data| ChatCompletionMessage {
                    role: data.role,
                    content: data.text,
                })
                .collect();
            (chat, messages)
        }
        SelectedType::Cancelled => return Ok(()),
    };

    print_stdin_help();

    // replay histories.
    for entry in &messages {
        match entry.role {
            MessageRole::System => (),
            MessageRole::User => {
                println_color("user:", disable_color)?;
                println!("{}", entry.content.trim());
            }
            MessageRole::Assistant => {
                println_color("assistant:", disable_color)?;
                println!("{}", entry.content.trim());
            }
        }
    }

    let mut read_buf = String::new();

    loop {
        let chat_completion_message = match messages.last() {
            Some(data) if data.role == MessageRole::User => {
                if disable_stream {
                    request_message(client.clone(), disable_color, model.clone(), &messages)?
                } else {
                    request_stream_message(client.clone(), disable_color, model.clone(), &messages)?
                }
            }
            Some(_) | None => {
                let chat_completion_message = match read_user_input(&mut read_buf, disable_color)? {
                    Some(data) => data,
                    None => return Ok(()),
                };

                if chat.title.is_empty() {
                    chat.title.clone_from(&chat_completion_message.content);
                    chat_repo.save_chat(&chat)?;
                }

                chat_completion_message
            }
        };

        let created_at = Utc::now();
        chat_repo.save_messages(
            &chat.chat_id,
            &[Message {
                message_id: MessageID(Uuid::new_v4()),
                created_at,
                updated_at: created_at,
                role: chat_completion_message.role.clone(),
                text: chat_completion_message.content.clone(),
            }],
        )?;

        messages.push(chat_completion_message);
    }
}

fn read_user_input(
    read_buf: &mut String,
    disable_color: bool,
) -> Fallible<Option<ChatCompletionMessage>> {
    read_buf.clear();

    println_color("user:", disable_color)?;

    stdin().read_to_string(read_buf)?;

    let content = read_buf.trim().to_owned();
    if content.is_empty() {
        return Ok(None);
    }

    Ok(Some(ChatCompletionMessage {
        role: MessageRole::User,
        content,
    }))
}

fn request_message(
    client: Client,
    disable_color: bool,
    model: ChatCompletionModel,
    messages: &[ChatCompletionMessage],
) -> Fallible<ChatCompletionMessage> {
    println_color("...", disable_color)?;

    let ret = client
        .post("https://api.openai.com/v1/chat/completions")
        .json(&ChatCompletionRequest {
            messages,
            model,
            ..Default::default()
        })
        .send()?;
    if let Err(e) = ret.error_for_status_ref() {
        return Err(e).with_context(|| {
            format!(
                "failed to request: {}",
                ret.text().unwrap_or_else(|e| format!("{:?}", e)),
            )
        });
    }

    let ret = ret.text()?;
    trace!(%ret);
    let mut ret = serde_json::from_str::<ChatCompletionResponse>(&ret)?;
    debug!(assistant = %serde_json::to_string_pretty(&ret)?);

    let answer = ret.choices.remove(0);
    println_color("assistant:", disable_color)?;
    println!("{}", answer.message.content.trim());
    if answer.finish_reason.is_none() {
        println_color("assistant: (in progress)", disable_color)?;
    }

    // other answers.
    for entry in &ret.choices {
        debug!(
            "assistant[{}]:{}",
            entry.index,
            entry.message.content.trim(),
        );
        if entry.finish_reason.is_none() {
            debug!("assistant[{}]: (in progress)", entry.index);
        }
    }

    Ok(answer.message)
}

#[instrument(skip_all)]
fn request_stream_message(
    client: Client,
    disable_color: bool,
    model: ChatCompletionModel,
    messages: &[ChatCompletionMessage],
) -> Fallible<ChatCompletionMessage> {
    println_color("...", disable_color)?;

    let res = client
        .post("https://api.openai.com/v1/chat/completions")
        .json(&ChatCompletionRequest {
            messages,
            stream: Some(true),
            max_tokens: Some(1000),
            model,
            ..Default::default()
        })
        .send()?;
    if let Err(e) = res.error_for_status_ref() {
        return Err(e).with_context(|| {
            format!(
                "failed to request: {}",
                res.text().unwrap_or_else(|e| format!("{:?}", e)),
            )
        });
    }

    let mut finish_reason = Option::<String>::None;
    let mut ret_message = String::new();
    let mut res = BufReader::new(res);
    let mut line = Vec::<u8>::new();
    loop {
        line.clear();
        match res.read_until(b'\n', &mut line) {
            Ok(0) => {
                warn!("EOF w/ new line");
                break;
            }
            Ok(_) => {
                if line.ends_with(b"\n") {
                    if line.starts_with(b"data: [DONE]") {
                        debug!("done");
                        break;
                    } else if line.starts_with(b"data:") {
                        let serialized =
                            serde_json::from_slice::<serde_json::Value>(&line["data:".len()..])
                                .with_context(|| format!("{:?}", line))?;
                        debug!(?serialized, "match");
                        let choice = serialized
                            .get("choices")
                            .context("{}.choices")?
                            .get(0)
                            .context("{}.choices[0]")?;
                        finish_reason = choice
                            .get("finish_reason")
                            .context("{}.choices[0].finish_reason")?
                            .as_str()
                            .map(str::to_owned);
                        match choice
                            .get("delta")
                            .context("{}.choices[0].delta")?
                            .get("content")
                        {
                            Some(data) => {
                                if ret_message.is_empty() {
                                    println_color("assistant:", disable_color)?;
                                }

                                let content =
                                    data.as_str().context("{}.choices[0].delta.content")?;
                                print!("{}", content);
                                stdout().flush()?;

                                ret_message += content;
                            }
                            None => debug!("{{}}.choices[0].delta.content is null"),
                        }
                    } else {
                        debug!(?line, "ignore");
                    }
                } else {
                    debug!("EOF w/o new line");
                    break;
                }
            }
            Err(e) => panic!("reader error: {:?}", e),
        }
    }

    if ret_message.is_empty() {
        bail!("ret is null. finish_reason: {:?}", finish_reason);
    } else {
        println!();
        if finish_reason != Some("stop".to_owned()) {
            info!(?finish_reason, "finish_reason != stop");
        }

        Ok(ChatCompletionMessage {
            role: MessageRole::Assistant,
            content: ret_message,
        })
    }
}

fn println_color(message: &str, disable_color: bool) -> Fallible<()> {
    if disable_color {
        println!("{}", message);
    } else {
        execute!(
            stdout(),
            SetBackgroundColor(Color::Green),
            SetForegroundColor(Color::White),
            Print(message),
            ResetColor,
        )?;
        println!();
    }

    Ok(())
}

/// Data structure for https://platform.openai.com/docs/api-reference/chat/create
/// ref. [ChatCompletionResponse]
#[derive(Default, Serialize)]
struct ChatCompletionRequest<'a> {
    model: ChatCompletionModel,
    messages: &'a [ChatCompletionMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
}

/// ref. [ChatCompletionRequest]
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
enum ChatCompletionModel {
    #[serde(rename = "gpt-3.5-turbo")]
    #[allow(dead_code)]
    GPT35Turbo,

    #[serde(rename = "gpt-3.5-turbo-16k")]
    #[allow(dead_code)]
    GPT35Turbo16k,

    #[serde(rename = "gpt-4")]
    #[allow(dead_code)]
    GPT4,

    #[serde(rename = "gpt-4-turbo")]
    #[allow(dead_code)]
    GPT4Turbo,

    #[serde(rename = "gpt-4o")]
    #[default]
    #[allow(dead_code)]
    GPT4o,

    /// for compatibility for old db https://platform.openai.com/docs/deprecations/
    #[serde(rename = "gpt-3.5-turbo-0301")]
    #[allow(dead_code)]
    GPT35Turbo0301,

    /// for compatibility for old db https://platform.openai.com/docs/deprecations/
    #[serde(rename = "gpt-3.5-turbo-0613")]
    #[allow(dead_code)]
    GPT35Turbo0613,
}

impl FromStr for ChatCompletionModel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "gpt-3.5-turbo" => Ok(Self::GPT35Turbo),
            "gpt-3.5-turbo-16k" => Ok(Self::GPT35Turbo16k),
            "gpt-3.5-turbo-0301" => Ok(Self::GPT35Turbo0301),
            "gpt-3.5-turbo-0613" => Ok(Self::GPT35Turbo0613),
            "gpt-4" => Ok(Self::GPT4),
            "gpt-4-turbo" => Ok(Self::GPT4Turbo),
            "gpt-4o" => Ok(Self::GPT4o),
            _ => bail!("unexpected str: {}", s),
        }
    }
}

/// ref. [ChatCompletionRequest]
#[derive(Deserialize, Serialize)]
struct ChatCompletionMessage {
    role: MessageRole,
    content: String,
}

/// Data structure for https://platform.openai.com/docs/api-reference/chat/create
///
/// ref. [ChatCompletionRequest]
#[derive(Deserialize, Serialize)]
struct ChatCompletionResponse {
    id: String,
    object: String,
    #[serde(with = "ts_seconds")]
    created: DateTime<Utc>,
    model: String,
    usage: ChatCompletionResponseUsage,
    /// ref. [ChatCompletionRequest::n]
    choices: Vec<ChatCompletionResponseChoice>,
}

/// ref. [ChatCompletionResponse]
#[derive(Deserialize, Serialize)]
struct ChatCompletionResponseUsage {
    prompt_tokens: u16,
    completion_tokens: u16,
    total_tokens: u16,
}

/// ref. [ChatCompletionResponse]
#[derive(Deserialize, Serialize)]
struct ChatCompletionResponseChoice {
    message: ChatCompletionMessage,
    finish_reason: Option<String>,
    index: usize,
}

/// refs.
/// - [ChatCompletionResponse]
/// - https://platform.openai.com/docs/guides/chat/response-format
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
enum ChatCompletionResponseChoiceFinishReason {
    Stop,
    Length,
    ContentFilter,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_completion_model_parse() {
        use ChatCompletionModel::*;

        match GPT35Turbo {
            GPT35Turbo => (),
            GPT35Turbo0301 => (),
            GPT35Turbo16k => {}
            GPT35Turbo0613 => {}
            GPT4 => {}
            GPT4Turbo => {}
            GPT4o => {}
        };

        #[allow(deprecated)]
        for entry in [
            GPT35Turbo,
            GPT35Turbo0301,
            GPT35Turbo16k,
            GPT35Turbo0613,
            GPT4,
            GPT4Turbo,
            GPT4o,
        ] {
            let serialized = get_serialized_string(&entry).unwrap();
            assert_eq!(entry, serialized.parse::<ChatCompletionModel>().unwrap());
        }
    }
}
