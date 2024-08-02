/*
 * Copyright 2024 sukawasatoru
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

use rust_myscript::prelude::*;

#[cfg(not(target_os = "windows"))]
use url::Url;

#[cfg(target_os = "windows")]
fn main() -> Fallible<()> {
    return Ok(());
}

#[cfg(not(target_os = "windows"))]
#[derive(clap::Parser)]
struct Opt {
    /// OpenTelemetry logs endpoint.
    otel_logs_endpoint: Option<Url>,
}

#[cfg(not(target_os = "windows"))]
#[tokio::main]
async fn main() -> Fallible<()> {
    use clap::Parser;
    use reqwest::Client;
    use rust_myscript::feature::otel::init_otel;
    use std::ffi::CString;

    let opt = Opt::parse();

    let _otel_guard = match opt.otel_logs_endpoint {
        Some(endpoint) => {
            let guard = init_otel(
                Client::new(),
                endpoint,
                env!("CARGO_PKG_NAME"),
                env!("CARGO_BIN_NAME"),
            )?;
            Some(guard)
        }
        None => {
            tracing_subscriber::fmt()
                .with_max_level(tracing_subscriber::FmtSubscriber::DEFAULT_MAX_LEVEL)
                .init();
            None
        }
    };

    let root_string = CString::new("/").expect("CString /");

    let (f_frsize, f_bavail, f_blocks) = unsafe {
        let mut root_stat = std::mem::zeroed();
        if libc::statvfs(root_string.as_ptr(), &mut root_stat) == 0 {
            (root_stat.f_frsize, root_stat.f_bavail, root_stat.f_blocks)
        } else {
            bail!("err: {}", std::io::Error::last_os_error());
        }
    };

    info!(event.name = "device.app.disk", f_frsize, f_bavail, f_blocks);

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
}
