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

use chrono::serde::ts_seconds;
use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand, ValueEnum};
use reqwest::blocking::Client;
use reqwest::header::{self, HeaderMap, HeaderValue};
use rust_myscript::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fmt::{Display, Formatter};
use std::io::prelude::*;
use std::io::Read;
use std::path::Path;
use std::str::FromStr;

/// Open AI API client - https://platform.openai.com/overview
#[derive(Parser)]
struct Opt {
    /// API Key for Open AI - https://platform.openai.com/account/api-keys
    #[arg(long, env = "OPENAI_CLI_API_KEY")]
    api_key: Option<String>,

    /// Organization ID for Open AI - https://platform.openai.com/account/org-settings
    #[arg(long, env = "OPENAI_CLI_ORG_ID")]
    org_id: Option<String>,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start chat interaction - https://platform.openai.com/docs/api-reference/chat
    #[command()]
    Chat {
        /// Model to use
        #[arg(long)]
        model: Option<String>,
    },

    /// Creates a new edit for the provided input, instruction, and parameters - https://platform.openai.com/docs/api-reference/edits/create
    #[command()]
    Edit {
        #[command(subcommand)]
        cmd: EditCommand,
    },

    /// Open editor to edit settings.
    #[command(subcommand)]
    Settings(SettingsCommand),
}

#[derive(Subcommand)]
enum EditCommand {
    /// Translate input text
    #[command()]
    Translate {
        /// Output language
        #[arg(long, default_value = "Japanese")]
        target: String,
    },
}

#[derive(Subcommand)]
enum SettingsCommand {
    /// List current settings.
    #[command()]
    List,

    /// Get current setting.
    #[command()]
    Get {
        /// Key to get the setting.
        key: SettingsKey,
    },

    /// Set setting.
    #[command()]
    Set {
        /// Key to set the setting.
        key: SettingsKey,

        /// Value to set the setting.
        value: String,
    },
}

#[derive(Clone, ValueEnum)]
enum SettingsKey {
    #[value(name = "organization_id")]
    OrganizationId,
    #[value(name = "api_key")]
    ApiKey,
}

impl Display for SettingsKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsKey::OrganizationId => f.write_str("organization_id"),
            SettingsKey::ApiKey => f.write_str("api_key"),
        }
    }
}

fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("hello");

    let project_dir = directories::ProjectDirs::from("com", "sukawasatoru", "OpenAI CLI")
        .context("no valid home directory")?;
    let config_dir = project_dir.config_dir();

    let opt = Opt::parse();

    match opt.cmd {
        Command::Chat { model } => chat(config_dir, opt.org_id, opt.api_key, model)?,
        Command::Edit { cmd } => match cmd {
            EditCommand::Translate { target } => {
                edit_translate(config_dir, opt.org_id, opt.api_key, target)?
            }
        },
        Command::Settings(cmd) => match cmd {
            SettingsCommand::List => list_settings(config_dir)?,
            SettingsCommand::Get { key } => get_setting(config_dir, &key)?,
            SettingsCommand::Set { key, value } => set_setting(config_dir, &key, value)?,
        },
    }

    info!("bye");

    Ok(())
}

fn chat(
    config_dir_path: &Path,
    arg_organization_id: Option<String>,
    arg_api_key: Option<String>,
    model: Option<String>,
) -> Fallible<()> {
    let default_headers = prepare_headers(config_dir_path, arg_organization_id, arg_api_key)?;

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

        eprintln!("user:");

        std::io::stdin().read_to_string(&mut read_buf)?;

        let content = read_buf.trim().to_owned();
        if content.is_empty() {
            break;
        }

        messages.push(ChatCompletionMessage {
            role: MessageRole::User,
            content,
        });

        eprintln!("...");

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
        eprintln!("assistant:\n{}", answer.message.content.trim());
        if answer.finish_reason.is_none() {
            eprintln!("assistant: (in progress)");
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

fn edit_translate(
    config_dir_path: &Path,
    arg_organization_id: Option<String>,
    arg_api_key: Option<String>,
    target: String,
) -> Fallible<()> {
    let default_headers = prepare_headers(config_dir_path, arg_organization_id, arg_api_key)?;

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(60 * 5))
        .default_headers(default_headers)
        .build()?;

    print_stdin_help();

    let mut read_buf = String::new();

    std::io::stdin().read_to_string(&mut read_buf)?;

    let content = read_buf.trim().to_owned();
    if content.is_empty() {
        return Ok(());
    }

    eprintln!("...");

    let body = json!({
        "model": EditModel::TextDavinchEdit001,
        "input": content,
        "instruction": format!("Translate to {target}"),
    });

    let ret = client
        .post("https://api.openai.com/v1/edits")
        .json(&body)
        .send()?
        .error_for_status()?
        .text()?;
    trace!(%ret);
    let ret = serde_json::from_str::<serde_json::Value>(&ret)?;
    let ret = ret["choices"][0]["text"]
        .as_str()
        .context("empty response text")?;
    eprintln!("{ret}");

    Ok(())
}

fn list_settings(config_dir_path: &Path) -> Fallible<()> {
    let settings = load_settings(config_dir_path)?;

    print_setting(&SettingsKey::OrganizationId, &settings.organization_id);
    print_setting(&SettingsKey::ApiKey, &settings.api_key);

    Ok(())
}

fn get_setting(config_dir_path: &Path, key: &SettingsKey) -> Fallible<()> {
    let settings = load_settings(config_dir_path)?;

    match *key {
        SettingsKey::OrganizationId => {
            print_setting(&SettingsKey::OrganizationId, &settings.organization_id)
        }
        SettingsKey::ApiKey => print_setting(&SettingsKey::ApiKey, &settings.api_key),
    }

    Ok(())
}

fn set_setting(config_dir_path: &Path, key: &SettingsKey, value: String) -> Fallible<()> {
    let mut settings = load_settings(config_dir_path)?;

    match *key {
        SettingsKey::OrganizationId => {
            settings.organization_id = Some(value);
        }
        SettingsKey::ApiKey => {
            settings.api_key = Some(value);
        }
    }

    save_settings(config_dir_path, &settings)?;

    Ok(())
}

fn print_setting(key: &SettingsKey, value: &Option<String>) {
    println!(
        "{key}: {}",
        value.as_ref().map(|data| data.as_str()).unwrap_or("(none)"),
    );
}

fn print_stdin_help() {
    #[cfg(target_os = "windows")]
    {
        eprintln!("Please input message and Ctrl+Z");
    }

    #[cfg(not(target_os = "windows"))]
    {
        eprintln!("Please input message and Ctrl+D");
    }
}

fn prepare_headers(
    config_dir_path: &Path,
    arg_organization_id: Option<String>,
    arg_api_key: Option<String>,
) -> Fallible<HeaderMap> {
    let settings = load_settings(config_dir_path)?;

    let mut default_headers = HeaderMap::new();

    let api_key = match arg_api_key.or(settings.api_key) {
        Some(data) => data,
        None => bail!("need api_key"),
    };

    let mut api_key = format!("Bearer {api_key}").parse::<HeaderValue>()?;
    api_key.set_sensitive(true);
    default_headers.insert(header::AUTHORIZATION, api_key);

    if let Some(organization_id) = arg_organization_id.or(settings.organization_id) {
        default_headers.insert("OpenAI-Organization", organization_id.parse()?);
    }

    Ok(default_headers)
}

fn load_settings(config_dir_path: &Path) -> Fallible<Settings> {
    if !config_dir_path.exists() {
        std::fs::create_dir_all(config_dir_path)?;
    }

    let file_path = config_dir_path.join("settings.toml");

    let mut settings_string = String::new();
    if file_path.exists() {
        let mut buf = std::io::BufReader::new(std::fs::File::open(file_path)?);
        buf.read_to_string(&mut settings_string)?;
    }

    Ok(toml::from_str(&settings_string)?)
}

fn save_settings(config_dir_path: &Path, settings: &Settings) -> Fallible<()> {
    if !config_dir_path.exists() {
        std::fs::create_dir_all(config_dir_path)?;
    }

    let mut buf = std::io::BufWriter::new(std::fs::File::create(
        config_dir_path.join("settings.toml"),
    )?);
    buf.write_all(toml::to_string(settings)?.as_bytes())?;
    buf.flush()?;

    Ok(())
}

#[derive(Deserialize, Serialize)]
struct Settings {
    organization_id: Option<String>,
    api_key: Option<String>,
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

/// ref. https://platform.openai.com/docs/api-reference/edits/create
#[derive(Clone, Serialize)]
enum EditModel {
    #[serde(rename = "text-davinci-edit-001")]
    TextDavinchEdit001,
    #[serde(rename = "code-davinci-edit-001")]
    CodeDavinciEdit001,
}

impl FromStr for EditModel {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "text-davinci-edit-001" => Ok(Self::TextDavinchEdit001),
            "code-davinci-edit-001" => Ok(Self::CodeDavinciEdit001),
            _ => bail!("unexpected str: {}", s),
        }
    }
}
