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

#[cfg_attr(not(windows), allow(dead_code))]
mod config;
#[cfg_attr(not(windows), allow(dead_code))]
mod model;

#[cfg(windows)]
mod registry;

use clap::Parser;
use url::Url;

#[derive(Debug, Parser)]
struct Opt {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Interval seconds between reports.
    #[arg(long)]
    interval_secs: Option<u64>,
    /// OpenTelemetry logs endpoint.
    #[arg(long)]
    otel_logs_endpoint: Option<Url>,
}

#[derive(Debug, clap::Subcommand)]
enum Cmd {
    /// Open the config file.
    Config,
}

#[cfg(not(windows))]
fn main() {
    eprintln!("crystal-disk-info-otel is supported on Windows only.");
    std::process::exit(1);
}

#[cfg(windows)]
#[tokio::main]
async fn main() -> rust_myscript::prelude::Fallible<()> {
    use crate::config::Config;
    use crate::model::disk_status_name;
    use crate::registry::REGISTRY_PATH;
    use rust_myscript::feature::otel::init_otel;
    use rust_myscript::prelude::*;
    use std::time::Duration;

    let opt = Opt::parse();

    match opt.cmd {
        Some(Cmd::Config) => {
            let path = Config::config_path()?;
            if !path.exists() {
                Config::default().save(&path)?;
            }
            opener::open(&path).with_context(|| format!("failed to open {}", path.display()))?;
            Ok(())
        }
        None => {
            let path = Config::config_path()?;
            let mut config = Config::load(&path)?;
            config.merge(&opt);
            config.save(&path)?;

            let endpoint = config.otel_logs_endpoint.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "otel_logs_endpoint is not configured. specify --otel-logs-endpoint or edit config via `crystal-disk-info-otel config`"
                )
            })?;

            let _otel_guard = init_otel(endpoint, env!("CARGO_PKG_NAME"), env!("CARGO_BIN_NAME"))?;

            let mut prev_last_update: Option<u32> = None;
            loop {
                match registry::read_snapshot() {
                    Ok(Some(snapshot)) => {
                        if prev_last_update == Some(snapshot.last_update) {
                            debug!("skip: LastUpdate not changed");
                        } else {
                            for disk in &snapshot.disks {
                                info!(
                                    event.name = "device.app.disk_status",
                                    disk.model_serial = disk.model_serial.as_str(),
                                    disk.model = disk.model.as_str(),
                                    disk.drive_letter = disk.drive_letter.as_str(),
                                    disk.size = disk.disk_size.as_str(),
                                    disk.temperature_celsius = disk.temperature_celsius,
                                    disk.temperature_class = disk.temperature_class.as_str(),
                                    disk.status = disk.disk_status,
                                    disk.status_name = disk_status_name(disk.disk_status),
                                );
                            }
                            prev_last_update = Some(snapshot.last_update);
                        }
                    }
                    Ok(None) => {
                        warn!(
                            registry_path = REGISTRY_PATH,
                            "CrystalDiskInfo registry key not found; is CrystalDiskInfo running with Gadget Support enabled?"
                        );
                    }
                    Err(e) => {
                        warn!(?e, registry_path = REGISTRY_PATH, "failed to read registry");
                    }
                }

                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(config.interval_secs)) => {}
                    _ = tokio::signal::ctrl_c() => break,
                }
            }

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Opt::command().debug_assert();
    }
}
