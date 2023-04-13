/*
 * Copyright 2023 sukawasatoru
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

use crate::data::repository::GetPreferencesRepository;
use crate::functions::{prepare_headers, print_stdin_help};
use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use crossterm::execute;
use crossterm::style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::tty::IsTty;
use reqwest::blocking::Client;
use rust_myscript::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::io::{stdin, stdout};
use std::str::FromStr;

pub fn chat<Ctx>(
    context: Ctx,
    arg_organization_id: Option<String>,
    arg_api_key: Option<String>,
    disable_color: bool,
    model: Option<String>,
) -> Fallible<()>
where
    Ctx: GetPreferencesRepository,
{
    let disable_color = disable_color || !stdin().is_tty();
    let default_headers = prepare_headers(&context, arg_organization_id, arg_api_key)?;

    let model = match model {
        Some(data) => data.parse::<ChatCompletionModel>()?,
        None => Default::default(),
    };

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(60 * 5))
        .default_headers(default_headers)
        .build()?;

    let mut messages = Vec::new();
    let mut read_buf = String::new();

    print_stdin_help();

    loop {
        read_buf.clear();

        println_color("user:", disable_color)?;

        stdin().read_to_string(&mut read_buf)?;

        let content = read_buf.trim().to_owned();
        if content.is_empty() {
            break;
        }

        messages.push(ChatCompletionMessage {
            role: MessageRole::User,
            content,
        });

        println_color("...", disable_color)?;

        let ret = client
            .post("https://api.openai.com/v1/chat/completions")
            .json(&ChatCompletionRequest {
                messages: &messages,
                max_tokens: Some(1000),
                n: Some(2),
                model: model.clone(),
                ..Default::default()
            })
            .send()?
            .error_for_status()?
            .text()?;
        trace!(%ret);
        let mut ret = serde_json::from_str::<ChatCompletionResponse>(&ret)?;
        debug!(assistant = %serde_json::to_string_pretty(&ret)?);

        let answer = ret.choices.remove(0);
        println_color("assistant:", disable_color)?;
        eprintln!("{}", answer.message.content.trim());
        if answer.finish_reason.is_none() {
            println_color("assistant: (in progress)", disable_color)?;
        }
        // use first answer to chat conversations.
        messages.push(answer.message);

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
    }

    Ok(())
}

fn println_color(message: &str, disable_color: bool) -> Fallible<()> {
    if disable_color {
        eprintln!("{}", message);
    } else {
        execute!(
            stdout(),
            SetBackgroundColor(Color::Green),
            SetForegroundColor(Color::White),
            Print(message),
            ResetColor,
        )?;
        eprintln!();
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
#[derive(Clone, Serialize)]
enum ChatCompletionModel {
    #[serde(rename = "gpt-3.5-turbo")]
    GPT35Turbo,
    #[serde(rename = "gpt-3.5-turbo-0301")]
    #[allow(dead_code)]
    GPT35Turbo0301,
}

impl FromStr for ChatCompletionModel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "gpt-3.5-turbo" => Ok(Self::GPT35Turbo),
            "gpt-3.5-turbo-0301" => Ok(Self::GPT35Turbo0301),
            _ => bail!("unexpected str: {}", s),
        }
    }
}

impl Default for ChatCompletionModel {
    fn default() -> Self {
        Self::GPT35Turbo
    }
}

/// ref. [ChatCompletionRequest]
#[derive(Deserialize, Serialize)]
struct ChatCompletionMessage {
    role: MessageRole,
    content: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum MessageRole {
    System,
    User,
    Assistant,
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
enum ChatCompletionResponseChoiceFinishReason {
    Stop,
    Length,
    ContentFilter,
}
