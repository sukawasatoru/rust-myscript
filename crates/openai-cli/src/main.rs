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

use crate::data::repository::{
    ChatRepositoryImpl, GetChatRepository, GetPreferencesRepository, PreferencesRepositoryImpl,
};
use crate::feature::{edit_translate, get_setting, list_settings, set_setting};
use crate::model::FileVersion;
use clap::{Parser, Subcommand, ValueEnum};
use rust_myscript::prelude::*;
use std::fmt::{Display, Formatter};

mod data;
mod feature;
mod functions;
mod model;

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
        /// Disable colored escape sequence.
        #[arg(long)]
        disable_color: bool,

        /// Disable ChatGPT like response.
        #[arg(long)]
        disable_stream: bool,

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
pub enum SettingsKey {
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

struct Context {
    chat_repo: ChatRepositoryImpl,
    prefs_repo: PreferencesRepositoryImpl,
}

impl GetChatRepository for Context {
    type Repo = ChatRepositoryImpl;

    fn get_chat_repo(&self) -> &Self::Repo {
        &self.chat_repo
    }
}

impl GetPreferencesRepository for Context {
    type Repo = PreferencesRepositoryImpl;

    fn get_prefs_repo(&self) -> &Self::Repo {
        &self.prefs_repo
    }
}

fn main() -> Fallible<()> {
    dotenv::dotenv().ok();

    if cfg!(target_os = "macos") {
        let log_dir_path = directories::UserDirs::new()
            .context("no valid home directory")?
            .home_dir()
            .join("Library/Logs/com.sukawasatoru.OpenAI CLI");
        let (non_blocking, guard) = tracing_appender::non_blocking(
            tracing_appender::rolling::hourly(log_dir_path, "openai-cli"),
        );
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(non_blocking)
            .init();

        std::mem::forget(guard);
    } else {
        tracing_subscriber::fmt::init();
    };

    info!("hello");

    let file_version = FileVersion::from([0, 1, 1]);

    let project_dir = directories::ProjectDirs::from("com", "sukawasatoru", "OpenAI CLI")
        .context("no valid home directory")?;
    let config_dir = project_dir.config_dir();

    let context = Context {
        chat_repo: ChatRepositoryImpl::create_with_path(&file_version, project_dir.data_dir())?,
        prefs_repo: PreferencesRepositoryImpl::create_with_path(config_dir.to_owned()),
    };

    let opt = Opt::parse();

    match opt.cmd {
        Command::Chat {
            disable_color,
            disable_stream,
            model,
        } => crate::feature::chat(
            context,
            opt.org_id,
            opt.api_key,
            disable_color,
            disable_stream,
            model,
        )?,
        Command::Edit { cmd } => match cmd {
            EditCommand::Translate { target } => {
                edit_translate(context, opt.org_id, opt.api_key, target)?
            }
        },
        Command::Settings(cmd) => match cmd {
            SettingsCommand::List => list_settings(context)?,
            SettingsCommand::Get { key } => get_setting(context, &key)?,
            SettingsCommand::Set { key, value } => set_setting(context, &key, value)?,
        },
    }

    info!("bye");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Opt::command().debug_assert();
    }

    #[ignore]
    #[test]
    fn opt_help() {
        Opt::command().print_help().unwrap();
        // Opt::command().get_matches_from(["foo", "chat", "--help"]);
    }
}
