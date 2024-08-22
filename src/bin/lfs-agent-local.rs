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

use clap::{Parser, Subcommand};
use directories::ProjectDirs;
use rust_myscript::feature::otel::init_otel;
use rust_myscript::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::fs::{create_dir_all, File};
use std::io::prelude::*;
use std::io::{BufReader, BufWriter};
use std::ops::DerefMut;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::LazyLock;
use url::Url;

/// LFS Custom Transfer Agent implementation that stores files to local storage.
#[derive(Parser)]
#[clap(name = "lfs-agent-local")]
struct Opt {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Install LFS Agent to the repository.
    Install {
        /// OpenTelemetry logs endpoint.
        #[arg(long)]
        otel_logs_endpoint: Option<Url>,

        /// A unique identifying name to distinct this repository.
        name: String,
    },
    /// This command invoked by Git client.
    Transfer {
        #[arg(long)]
        otel_logs_endpoint: Option<Url>,
        value: String,
    },
}

/// https://github.com/git-lfs/git-lfs/blob/main/docs/custom-transfers.md
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "event")]
enum ProtocolRequest {
    #[allow(unused)]
    Init {
        operation: ProtocolRequestOperation,
        remote: String,
        concurrent: bool,
        #[serde(rename = "concurrenttransfers")]
        concurrent_transfers: u16,
    },
    Download {
        oid: String,
        size: u64,
        // action always null.
    },
    Upload {
        oid: String,
        size: u64,
        path: PathBuf,
        // action always null.
    },
    Terminate,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProtocolRequestOperation {
    Download,
    Upload,
}

/// https://github.com/git-lfs/git-lfs/blob/main/docs/custom-transfers.md
#[cfg_attr(test, derive(Deserialize))]
#[derive(Serialize)]
#[serde(tag = "event")]
enum ProtocolResponse {
    #[serde(rename = "complete")]
    UploadComplete { oid: String },
    #[serde(rename = "complete")]
    DownloadComplete { oid: String, path: PathBuf },
    #[serde(rename = "progress")]
    Progress {
        oid: String,
        #[serde(rename = "bytesSoFar")]
        bytes_so_far: u64,
        #[serde(rename = "bytesSinceLast")]
        bytes_since_last: u64,
    },
    #[serde(rename = "complete")]
    Error {
        oid: String,
        error: ProtocolResponseErrorBody,
    },
}

#[cfg_attr(test, derive(Deserialize))]
#[derive(Serialize)]
struct ProtocolResponseFatalError {
    error: ProtocolResponseErrorBody,
}

#[cfg_attr(test, derive(Deserialize))]
#[derive(Serialize)]
struct ProtocolResponseErrorBody {
    code: u16,
    message: String,
}

#[tokio::main]
async fn main() -> Fallible<()> {
    let opt = Opt::parse();

    match opt.cmd {
        Cmd::Install {
            otel_logs_endpoint,
            name,
        } => install(otel_logs_endpoint, &name)?,
        Cmd::Transfer {
            otel_logs_endpoint,
            value,
        } => {
            let _guard = otel_logs_endpoint.and_then(|endpoint| {
                init_otel(
                    reqwest::Client::new(),
                    endpoint,
                    env!("CARGO_PKG_NAME"),
                    env!("CARGO_BIN_NAME"),
                )
                .ok()
            });

            tokio::task::spawn_blocking(move || {
                struct Context;
                impl DefaultLocalFileDataSource for Context {}
                impl DefaultProjectDirectories for Context {}

                transfer(
                    Context,
                    &mut std::io::stdin(),
                    &mut std::io::stdout(),
                    &value,
                )
            })
            .await??
        }
    }

    Ok(())
}

/// - https://github.com/git-lfs/git-lfs/blob/fc61feb/docs/custom-transfers.md#using-a-custom-transfer-type-without-the-api-server
/// - https://github.com/git-lfs/git-lfs/blob/fc61feb/docs/custom-transfers.md#defining-a-custom-transfer-type
fn install(otel_logs_endpoint: Option<Url>, name: &str) -> Fallible<()> {
    let ret = std::process::Command::new("git")
        .args(["config", "lfs.standalonetransferagent", "lfs-agent-local"])
        .spawn()?
        .wait_with_output()?;
    ensure!(ret.status.success(), "failed to execute git command");

    let ret = std::process::Command::new("git")
        .args([
            "config",
            "lfs.customtransfer.lfs-agent-local.path",
            "lfs-agent-local",
        ])
        .spawn()?
        .wait_with_output()?;
    ensure!(ret.status.success(), "failed to execute git command");

    let value = match otel_logs_endpoint {
        Some(target) => format!(r#"transfer --otel-logs-endpoint "{target}" "{name}""#),
        None => format!(r#"transfer "{name}""#),
    };
    let ret = std::process::Command::new("git")
        .args(["config", "lfs.customtransfer.lfs-agent-local.args", &value])
        .spawn()?
        .wait_with_output()?;
    ensure!(ret.status.success(), "failed to execute git command");

    Ok(())
}

fn transfer<Ctx, R, W>(ctx: Ctx, reader: &mut R, mut writer: &mut W, name: &str) -> Fallible<()>
where
    Ctx: GetLocalFileDataSource,
    Ctx: GetProjectDirectories,
    R: Read,
    W: Write,
{
    let pid = std::process::id();
    let repo_data_dir = ctx.get_project_directories().get_data_dir().join(name);

    let mut reader = BufReader::new(reader);
    let mut buf = String::new();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => {
                warn!(pid, "stdin EOS");
                break;
            }
            Ok(_) => {
                debug!(pid, line = %buf);
                if buf.trim().is_empty() {
                    continue;
                }

                let req = match serde_json::from_str::<ProtocolRequest>(&buf) {
                    Ok(data) => data,
                    Err(e) => {
                        error!(pid, ?e, line = buf, "failed to parse line");
                        let res = ProtocolResponseFatalError {
                            error: ProtocolResponseErrorBody {
                                code: 1,
                                message: "failed to parse line".to_owned(),
                            },
                        };

                        writeln!(writer, "{}", serde_json::to_string(&res)?)?;
                        writer.flush()?;
                        bail!("failed to parse line: {}", e);
                    }
                };

                match req {
                    ProtocolRequest::Init { .. } => {
                        // https://github.com/git-lfs/git-lfs/blob/fc61feb/docs/custom-transfers.md#stage-1-initiation
                        writeln!(writer, "{}", serde_json::to_string(&json!({}))?)?;
                        writer.flush()?;
                    }
                    ProtocolRequest::Terminate => {
                        // https://github.com/git-lfs/git-lfs/blob/fc61feb/docs/custom-transfers.md#stage-3-finish--cleanup
                        info!(pid, "all transfers have been processed");
                        break;
                    }
                    ProtocolRequest::Upload { oid, size, path } => {
                        transfer_upload(&ctx, pid, &repo_data_dir, &mut writer, oid, size, &path)?;
                    }
                    ProtocolRequest::Download { oid, size } => {
                        transfer_download(&ctx, pid, &repo_data_dir, &mut writer, oid, size)?;
                    }
                }
            }
            Err(e) => {
                error!(pid, ?e, "failed to read line");
                let res = ProtocolResponseFatalError {
                    error: ProtocolResponseErrorBody {
                        code: 1,
                        message: "failed to read line".to_owned(),
                    },
                };

                writeln!(writer, "{}", serde_json::to_string(&res)?)?;
                writer.flush()?;
                bail!("failed to read line: {}", e);
            }
        }
    }

    Ok(())
}

/// - https://github.com/git-lfs/git-lfs/blob/fc61feb/docs/custom-transfers.md#uploads
/// - https://github.com/git-lfs/git-lfs/blob/fc61feb/docs/custom-transfers.md#progress
fn transfer_upload<Ctx: GetLocalFileDataSource, W: Write>(
    ctx: &Ctx,
    pid: u32,
    repo_data_dir: &Path,
    mut result_writer: &mut W,
    oid: String,
    size: u64,
    source_pathname: &Path,
) -> Fallible<()> {
    let target_pathname = repo_data_dir.join(create_object_path_from_oid(&oid));
    let data_source = ctx.get_local_file_data_source();

    info!(
        pid,
        oid,
        source_pathname = %source_pathname.display(),
        target_pathname = %target_pathname.display(),
        "transfer start",
    );

    struct TransferContainer<W: Write> {
        pid: u32,
        oid: String,
        write_total: Cell<u64>,
        result_writer: RefCell<W>,
    }

    impl<W: Write> TransferContainer<W> {
        fn cb(&self, write_count: usize) {
            let write_count = match write_count.try_into() {
                Ok(data) => data,
                Err(e) => {
                    warn!(self.pid, ?e, write_count, "failed to convert usize to u64");
                    return;
                }
            };

            let write_total = self.write_total.get() + write_count;
            self.write_total.set(write_total);

            let res = match serde_json::to_string(&ProtocolResponse::Progress {
                oid: self.oid.clone(),
                bytes_so_far: write_total,
                bytes_since_last: write_count,
            }) {
                Ok(data) => data,
                Err(e) => {
                    warn!(
                        self.pid,
                        ?e,
                        write_count,
                        write_total,
                        "failed to serialize a value to json",
                    );
                    return;
                }
            };

            info!(self.pid, write_count, write_total, "progress");
            if let Err(e) = writeln!(self.result_writer.borrow_mut(), "{res}") {
                warn!(
                    self.pid,
                    ?e,
                    write_count,
                    write_total,
                    "failed to write to stdout",
                );
            }
        }
    }

    let container = Rc::new(TransferContainer {
        pid,
        oid: oid.clone(),
        write_total: Cell::new(0),
        result_writer: RefCell::new(&mut result_writer),
    });
    let container_for_lambda = container.clone();

    if let Err(e) = data_source.copy_file(
        source_pathname,
        &target_pathname,
        move |write_count: usize| container_for_lambda.cb(write_count),
    ) {
        warn!(pid, ?e, "failed to transfer file");
        let res = serde_json::to_string(&ProtocolResponse::Error {
            oid,
            error: ProtocolResponseErrorBody {
                code: 1,
                message: "failed to transfer file".to_owned(),
            },
        })?;
        let mut result_writer = container.result_writer.borrow_mut();
        let result_writer = result_writer.deref_mut();
        writeln!(result_writer, "{res}")?;
        result_writer.deref_mut().flush()?;
        return Ok(());
    }

    let res = if size == container.write_total.get() {
        info!(pid, oid, "complete");
        ProtocolResponse::UploadComplete { oid }
    } else {
        warn!(
            pid,
            size,
            write_total = container.write_total.get(),
            oid,
            "complete but unexpected size",
        );
        ProtocolResponse::Error {
            oid,
            error: ProtocolResponseErrorBody {
                code: 1,
                message: "unexpected size".to_owned(),
            },
        }
    };

    writeln!(result_writer, "{}", serde_json::to_string(&res)?)?;
    result_writer.flush()?;

    Ok(())
}

/// - https://github.com/git-lfs/git-lfs/blob/fc61feb/docs/custom-transfers.md#downloads
/// - https://github.com/git-lfs/git-lfs/blob/fc61feb/docs/custom-transfers.md#progress
fn transfer_download<Ctx: GetLocalFileDataSource, W: Write>(
    ctx: &Ctx,
    pid: u32,
    repo_data_dir: &Path,
    result_writer: &mut W,
    oid: String,
    size: u64,
) -> Fallible<()> {
    let source_pathname = repo_data_dir.join(create_object_path_from_oid(&oid));
    let local_file_data_source = ctx.get_local_file_data_source();

    match local_file_data_source.get_file_size(&source_pathname) {
        Some(storage_file_size) if storage_file_size == size => {
            let res = serde_json::to_string(&ProtocolResponse::DownloadComplete {
                oid,
                path: if cfg!(target_os = "windows") {
                    PathBuf::from(source_pathname.display().to_string().replace('\\', "/"))
                } else {
                    source_pathname
                },
            })?;
            writeln!(result_writer, "{res}")?;
        }
        Some(storage_file_size) => {
            warn!(pid, oid, source_pathname = %source_pathname.display(), storage_file_size, size, "unexpected file size");
            let res = serde_json::to_string(&ProtocolResponse::Error {
                oid,
                error: ProtocolResponseErrorBody {
                    code: 1,
                    message: "unexpected file size".to_string(),
                },
            })?;
            writeln!(result_writer, "{res}")?;
        }
        None => {
            warn!(pid, oid, source_pathname = %source_pathname.display(), "source not found");
            let res = serde_json::to_string(&ProtocolResponse::Error {
                oid,
                error: ProtocolResponseErrorBody {
                    code: 1,
                    message: "source not found".to_string(),
                },
            })?;
            writeln!(result_writer, "{res}")?;
        }
    }

    Ok(())
}

fn create_object_path_from_oid(oid: &str) -> PathBuf {
    PathBuf::new().join(&oid[0..=1]).join(&oid[2..=3]).join(oid)
}

#[cfg_attr(test, mockall::automock)]
trait LocalFileDataSource {
    #[cfg_attr(test, mockall::concretize)]
    fn copy_file<Cb: Fn(usize)>(
        &self,
        source: &Path,
        target: &Path,
        progress_cb: Cb,
    ) -> Fallible<()> {
        let mut reader = BufReader::new(File::open(source).context("failed to open source file")?);

        create_dir_all(target.parent().context("no parent")?)
            .context("failed to create data directory")?;

        let mut writer =
            BufWriter::new(File::create(target).context("failed to create a file to destination")?);
        let mut buf = [0u8; 4096];

        loop {
            let read_count = reader.read(&mut buf).context("failed to read file")?;
            if read_count == 0 {
                break;
            }

            writer.write_all(&buf[0..read_count])?;
            progress_cb(read_count);
        }

        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    fn get_file_size(&self, pathname: &Path) -> Option<u64> {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(pathname).ok().map(|data| data.size())
    }

    #[cfg(target_os = "windows")]
    fn get_file_size(&self, pathname: &Path) -> Option<u64> {
        use std::os::windows::fs::MetadataExt;
        std::fs::metadata(pathname)
            .ok()
            .map(|data| data.file_size())
    }
}

trait DefaultLocalFileDataSource {}

impl<T: DefaultLocalFileDataSource> LocalFileDataSource for T {}

trait GetLocalFileDataSource {
    type DataSource: LocalFileDataSource;

    fn get_local_file_data_source(&self) -> &Self::DataSource;
}

impl<T: DefaultLocalFileDataSource> GetLocalFileDataSource for T {
    type DataSource = Self;

    fn get_local_file_data_source(&self) -> &Self::DataSource {
        self
    }
}

static PROJECT_DIR: LazyLock<ProjectDirs> = LazyLock::new(|| {
    ProjectDirs::from("com", "sukawasatoru", "LFS Agent Local").expect("no valid home directory")
});

#[cfg_attr(test, mockall::automock)]
trait ProjectDirectories {
    fn get_data_dir(&self) -> &Path {
        PROJECT_DIR.data_dir()
    }
}

trait DefaultProjectDirectories {}

impl<T: DefaultProjectDirectories> ProjectDirectories for T {}

trait GetProjectDirectories {
    type PJDirs: ProjectDirectories;

    fn get_project_directories(&self) -> &Self::PJDirs;
}

impl<T: DefaultProjectDirectories> GetProjectDirectories for T {
    type PJDirs = Self;

    fn get_project_directories(&self) -> &Self::PJDirs {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use mockall::predicate::*;
    use std::io::Cursor;

    #[test]
    fn verify_cli() {
        Opt::command().debug_assert();
    }

    #[test]
    fn create_oid_succeeded() {
        let actual = create_object_path_from_oid(
            "ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb",
        );
        let expected =
            Path::new("ff/66/ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb");
        assert_eq!(actual, expected);
    }

    #[test]
    fn request_init_download() {
        struct MockContext {
            local_file_data_source: MockLocalFileDataSource,
            project_directories: MockProjectDirectories,
        }

        impl GetLocalFileDataSource for MockContext {
            type DataSource = MockLocalFileDataSource;

            fn get_local_file_data_source(&self) -> &Self::DataSource {
                &self.local_file_data_source
            }
        }

        impl GetProjectDirectories for MockContext {
            type PJDirs = MockProjectDirectories;

            fn get_project_directories(&self) -> &Self::PJDirs {
                &self.project_directories
            }
        }

        let mut input = Cursor::new(
            r#"
{"event":"init","operation":"download","remote":"origin","concurrent":true,"concurrenttransfers":8}
{"event":"download","oid":"ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb","size":80,"action":null}
{"event":"terminate"}
"#,
        );

        let mut project_directories = MockProjectDirectories::default();
        project_directories
            .expect_get_data_dir()
            .return_const(Path::new("/test-target").to_owned());

        let mut local_file_data_source = MockLocalFileDataSource::default();
        local_file_data_source
            .expect_get_file_size()
            .with(eq(Path::new("/test-target/test-repo-name/ff/66/ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb")))
            .return_const(80);

        let mut out = vec![];
        let context = MockContext {
            local_file_data_source,
            project_directories,
        };
        transfer(context, &mut input, &mut out, "test-repo-name").unwrap();

        let mut lines = out.lines();

        // event: init
        assert_eq!(lines.next().unwrap().unwrap(), "{}");

        // event: download
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&lines.next().unwrap().unwrap()).unwrap(),
            json!({
                "event": "complete",
                "oid": "ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb",
                "path": "/test-target/test-repo-name/ff/66/ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb",
            })
        );

        // event: terminate
        assert!(lines.next().is_none());
    }

    #[test]
    fn request_init_upload() {
        struct MockContext {
            local_file_data_source: MockLocalFileDataSource,
            project_directories: MockProjectDirectories,
        }

        impl GetLocalFileDataSource for MockContext {
            type DataSource = MockLocalFileDataSource;

            fn get_local_file_data_source(&self) -> &Self::DataSource {
                &self.local_file_data_source
            }
        }

        impl GetProjectDirectories for MockContext {
            type PJDirs = MockProjectDirectories;

            fn get_project_directories(&self) -> &Self::PJDirs {
                &self.project_directories
            }
        }

        let mut project_directories = MockProjectDirectories::default();
        project_directories
            .expect_get_data_dir()
            .return_const(Path::new("/test-target").to_owned());

        let mut local_file_data_source = MockLocalFileDataSource::default();
        local_file_data_source
            .expect_copy_file()
            .withf(|source, dest, _| {
                source == Path::new("/Users/user/tmp/.git/lfs/objects/ff/66/ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb") &&
                  dest == Path::new("/test-target/test-repo-name/ff/66/ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb")
            })
            .returning(|_, _, cb| {
                cb(4096);
                cb(10);
                Ok(())
            });

        let mut input = Cursor::new(
            r#"
{"event":"init","operation":"upload","remote":"origin","concurrent":true,"concurrenttransfers":8}
{"event":"upload","oid":"ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb","size":4106,"path":"/Users/user/tmp/.git/lfs/objects/ff/66/ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb","action":null}
{"event":"terminate"}
"#,
        );

        let mut out = vec![];
        let context = MockContext {
            local_file_data_source,
            project_directories,
        };
        transfer(context, &mut input, &mut out, "test-repo-name").unwrap();

        let mut lines = out.lines();

        // event: init
        assert_eq!(lines.next().unwrap().unwrap(), "{}");

        // event: upload (progress)
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&lines.next().unwrap().unwrap()).unwrap(),
            json!({
                "event": "progress",
                "oid": "ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb",
                "bytesSoFar": 4096,
                "bytesSinceLast": 4096,
            })
        );
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&lines.next().unwrap().unwrap()).unwrap(),
            json!({
                "event": "progress",
                "oid": "ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb",
                "bytesSoFar": 4106,
                "bytesSinceLast": 10,
            })
        );

        // event: upload (complete)
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&lines.next().unwrap().unwrap()).unwrap(),
            json!({
                "event": "complete",
                "oid": "ff664e5803ae941f7b490e4affc4be0a8ba8b8954608f31f5e29bcdce840f5cb",
            })
        );

        // event: terminate
        assert!(lines.next().is_none());
    }

    #[test]
    fn request_invalid_json() {
        let mut input = Cursor::new("{aaaaaaa: bbbb}");
        let mut out = vec![];

        let local_file_data_source = MockLocalFileDataSource::default();
        struct MockContext {
            local_file_data_source: MockLocalFileDataSource,
        }
        impl GetLocalFileDataSource for MockContext {
            type DataSource = MockLocalFileDataSource;

            fn get_local_file_data_source(&self) -> &Self::DataSource {
                &self.local_file_data_source
            }
        }
        impl DefaultProjectDirectories for MockContext {}

        let context = MockContext {
            local_file_data_source,
        };
        let ret = transfer(context, &mut input, &mut out, "test-repo-name");
        assert!(ret.is_err());

        let mut lines = out.lines();
        serde_json::from_str::<ProtocolResponseFatalError>(&lines.next().unwrap().unwrap())
            .unwrap();
        assert!(lines.next().is_none());
    }
}
