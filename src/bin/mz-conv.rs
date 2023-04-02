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

use clap::{Parser, Subcommand};
use rust_myscript::prelude::*;
use std::io::prelude::*;
use std::path::{Path, PathBuf};

/// Encrypt/Decrypt files for MZ.
#[derive(Parser)]
struct Opt {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Encrypt files.
    Encrypt {
        /// Input file or directory.
        input: PathBuf,

        /// Output file or directory.
        output: PathBuf,
    },

    /// Decrypt files.
    Decrypt {
        /// Input file or directory.
        input: PathBuf,

        /// Output file or directory.
        output: PathBuf,
    },
}

#[allow(dead_code)]
const DEFAULT_SIGNATURE: &str = "5250474d56000000";
#[allow(dead_code)]
const DEFAULT_VERSION: &str = "000301";
#[allow(dead_code)]
const DEFAULT_REMAIN: &str = "0000000000";
const DEFAULT_HEADER_LEN: usize = 16;
const PNG_HEADER_BYTES: [u8; DEFAULT_HEADER_LEN] = [
    0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
    0x52,
];

fn main() -> Fallible<()> {
    dotenv::dotenv().ok();

    tracing_subscriber::fmt::init();

    info!("hello");

    let opt = Opt::parse();

    match opt.cmd {
        Command::Encrypt { .. } => {
            todo!()
        }
        Command::Decrypt { input, output } => decrypt(&input, &output)?,
    }

    info!("bye");
    Ok(())
}

fn decrypt(input_path: &Path, output_path: &Path) -> Fallible<()> {
    let input_path = input_path.canonicalize()?;

    if !input_path.exists() {
        bail!("no such file/directory: {}", input_path.display());
    }

    if input_path.is_file() {
        if output_path.exists() {
            bail!(
                "specified output path has already file/directory: {}",
                output_path.display()
            );
        }
    } else if output_path.exists() {
        if output_path.is_file() {
            bail!(
                "specified output path has already directory: {}",
                output_path.display()
            );
        }
    } else {
        std::fs::create_dir_all(output_path)?;
    }

    let output_path = output_path.canonicalize()?;

    let dir_file_paths = walk_dir(&input_path)?;
    for file_path in dir_file_paths {
        if let Some(extension) = file_path.extension() {
            if extension.to_string_lossy().to_ascii_lowercase() == "png_" {
                info!("{}", file_path.display());

                let mut output_path =
                    generate_output_filepath(&input_path, &output_path, &file_path)?;
                output_path.set_extension("png");
                decrypt_png_file(&file_path, &output_path)?;
                println!("{}", output_path.display());
            } else {
                info!("ignore unsupported file: {}", file_path.display());
            }
        } else {
            info!("unknown file: {}", file_path.display());
        }
    }

    Ok(())
}

fn walk_dir(dir_path: &Path) -> Fallible<Vec<PathBuf>> {
    let dir_entries = std::fs::read_dir(dir_path)?;
    let mut file_paths = vec![];
    for entry in dir_entries {
        let entry = entry?;
        let entry_path = entry.path();
        if entry_path.is_file() {
            file_paths.push(entry_path);
        } else {
            let mut child = walk_dir(&entry_path)?;
            file_paths.append(&mut child);
        }
    }

    Ok(file_paths)
}

fn generate_output_filepath(
    input_dir: &Path,
    target_dir: &Path,
    source: &Path,
) -> Fallible<PathBuf> {
    Ok(target_dir.join(source.strip_prefix(input_dir)?))
}

fn decrypt_png_file(input_path: &Path, output_path: &Path) -> Fallible<()> {
    if !input_path.is_file() {
        bail!("specified input path is not file: {}", input_path.display());
    }

    if output_path.exists() {
        bail!(
            "specified output path has already file/directory: {}",
            output_path.display()
        );
    }

    let parent_dir = output_path.parent().context("no parent")?;
    if !parent_dir.exists() {
        std::fs::create_dir_all(parent_dir)?;
    }

    let mut input = vec![];
    let mut reader = std::io::BufReader::new(std::fs::File::open(input_path)?);
    reader.read_to_end(&mut input)?;

    let mut writer = std::io::BufWriter::new(std::fs::File::create(output_path)?);

    // write png header instead.
    writer.write_all(&PNG_HEADER_BYTES[..])?;

    // DEFAULT_HEADER_LEN * 2 for remove encrypted 16bytes.
    // write input w/o header.
    writer.write_all(&input[DEFAULT_HEADER_LEN * 2..])?;

    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_output_filepath_() {
        let input_dir = Path::new("/path/to/app");
        let target_dir = Path::new("/path/to/output");
        let source = Path::new("/path/to/app/img/picture/img.png_");
        let actual = generate_output_filepath(input_dir, target_dir, source).unwrap();
        assert_eq!(
            "/path/to/output/img/picture/img.png_",
            &actual.display().to_string(),
        );
    }
}
