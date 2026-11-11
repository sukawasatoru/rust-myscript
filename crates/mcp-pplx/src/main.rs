/*
 * Copyright 2026 sukawasatoru
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

//! CLI entry point: parse commands, initialize logging, and dispatch library features.

mod cli;

use clap::Parser;
use cli::{AuthCommand, Command, Opt};
use mcp_pplx::feature::{auth, mcp};
use rust_myscript::prelude::*;
use tracing::Level;

#[tokio::main]
async fn main() -> Fallible<()> {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_writer(std::io::stderr)
        .init();

    let opt = Opt::parse();
    // Credential backends and terminal input are synchronous.
    let api_key = tokio::task::spawn_blocking(move || -> Fallible<Option<String>> {
        match opt.command {
            Command::Serve { api_key } => auth::resolve_api_key(api_key).map(Some),
            Command::Auth { command } => {
                match command {
                    AuthCommand::Set { stdin } => auth::set(stdin)?,
                    AuthCommand::Delete => auth::delete()?,
                }
                Ok(None)
            }
        }
    })
    .await??;
    if let Some(api_key) = api_key {
        mcp::run(api_key).await;
    }
    Ok(())
}
