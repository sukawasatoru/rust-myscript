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

mod cli;

use clap::Parser;
use cli::{Cli, Command};
use dat_explorer::{application::thread_service::ThreadService, data::five_ch::subject};
use tracing::Level;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Viewer { dat_dir } => {
            if let Err(e) = run_viewer(dat_dir).await {
                eprintln!("{e:#}");
                std::process::exit(1);
            }
        }
        Command::Mcp {
            dat_dir,
            disable_body_limit,
        } => {
            tracing_subscriber::fmt()
                .with_max_level(Level::INFO)
                .with_writer(std::io::stderr)
                .init();
            let http_client = match subject::build_client() {
                Ok(client) => client,
                Err(e) => {
                    tracing::error!(?e, "failed to initialize HTTP client");
                    return;
                }
            };
            let service = ThreadService::new(dat_dir, http_client);
            dat_explorer::feature::mcp::run(service, disable_body_limit).await;
        }
    }
}

async fn run_viewer(dat_dir: std::path::PathBuf) -> anyhow::Result<()> {
    let dat_dir = std::fs::canonicalize(dat_dir)?;
    anyhow::ensure!(dat_dir.is_dir(), "DAT_DIR must be a directory");
    let service = std::sync::Arc::new(ThreadService::new(
        dat_dir.clone(),
        subject::build_client()?,
    ));
    let store = dat_explorer::data::viewer_store::ViewerStore::discover()?;
    dat_explorer::feature::viewer::run(dat_dir, service, store).await
}
