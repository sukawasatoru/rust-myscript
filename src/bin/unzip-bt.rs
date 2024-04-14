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

use clap::{Parser, ValueHint};
use indicatif::HumanDuration;
use rust_myscript::prelude::*;
use std::fs::File;
use std::io::prelude::*;
use std::io::{BufReader, Cursor};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use zip::result::ZipError;
use zip::ZipArchive;

#[derive(Parser)]
struct Opt {
    #[clap(short, long, default_value = "200000")]
    update_interval: usize,

    #[clap(short, long, default_value_t = num_cpus::get() + 1)]
    threads: usize,

    #[clap(short, long, default_value = "0")]
    start_length: usize,

    /// Zip file.
    #[clap(value_hint = ValueHint::FilePath)]
    file: PathBuf,
}

fn main() -> Fallible<()> {
    let opt = Opt::parse();

    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let mut file_indexes = vec![];

    let mut file_vec = vec![];
    {
        BufReader::new(File::open(&opt.file)?).read_to_end(&mut file_vec)?;
    }
    let mut zip_archive = ZipArchive::new(Cursor::new(&file_vec))?;
    for index in 0..zip_archive.len() {
        let entry = match zip_archive.by_index(index) {
            Ok(data) => data,
            Err(ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED)) => {
                info!(%index, "password required");
                file_indexes.push(index);
                continue;
            }
            Err(e) => Err(e).context("failed to get file")?,
        };

        if entry.is_dir() {
            debug!(entry = ?entry.enclosed_name(), "directory");
        }
    }

    let (tx, rx) = tokio::sync::watch::channel(false);
    let next_base_password = Arc::new(next_password_generator(opt.start_length));

    let start_time = std::time::Instant::now();
    let bars = indicatif::MultiProgress::new();
    let mut threads = vec![];
    let index = match file_indexes.first() {
        Some(data) => *data,
        None => return Ok(()),
    };
    for i in 0..opt.threads {
        let tx = tx.clone();
        let rx = rx.clone();
        let next_password = next_base_password.clone();
        let update_interval = opt.update_interval;
        let bar = indicatif::ProgressBar::new_spinner().with_message(format!("{i}"));
        bars.add(bar.clone());
        let mut zip_archive = ZipArchive::new(Cursor::new(file_vec.clone()))?;
        let t = std::thread::spawn(move || {
            let mut update_counter = 0usize;
            let mut password = next_password(None);
            let mut buf = vec![];
            loop {
                if rx.has_changed().unwrap_or(true) {
                    bar.finish();
                    break;
                }

                update_counter += 1;
                if update_interval < update_counter {
                    update_counter = 0;
                    bar.set_message(format!(
                        "{bytes} {password}",
                        bytes = password
                            .iter()
                            .map(|data| format!("{:03}", data))
                            .collect::<Vec<_>>()
                            .join(", "),
                        password = String::from_utf8_lossy(&password),
                    ));
                    bar.tick();
                }

                match zip_archive.by_index_decrypt(index, &password) {
                    Ok(Ok(mut entry)) => {
                        buf.clear();
                        if entry.read_to_end(&mut buf).is_ok() {
                            bar.finish_with_message(format!(
                                "{bytes} {password} ✅",
                                bytes = password
                                    .iter()
                                    .map(|data| format!("{:03}", data))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                password = String::from_utf8_lossy(&password),
                            ));
                            tx.send(true).ok();
                            break;
                        }
                    }
                    Ok(Err(_)) => {}
                    Err(e) => {
                        bar.abandon_with_message(format!("{:?}", e));
                        tx.send(true).ok();
                        break;
                    }
                }
                password = next_password(Some(password));
            }
        });
        threads.push(t);
    }

    for t in threads {
        t.join().unwrap();
    }

    println!("{}", HumanDuration(start_time.elapsed()));

    Ok(())
}

fn next_password_generator(start_length: usize) -> impl Fn(Option<Vec<u8>>) -> Vec<u8> {
    fn inc_carry_up(start_index: usize, end_index: usize, password: &mut [u8]) -> (bool, bool) {
        for (index, entry) in password
            .iter_mut()
            .enumerate()
            .take(end_index + 1)
            .skip(start_index)
        {
            if entry == &b'~' {
                *entry = b' ';
                if index == end_index {
                    return (true, true);
                }
            } else {
                *entry += 1;
                return (false, index == end_index);
            }
        }
        (false, true)
    }

    let base_password = Mutex::new(vec![]);
    move |password: Option<Vec<u8>>| match password {
        Some(mut password) => {
            let ret = inc_carry_up(0, password.len() - 1, &mut password);
            match ret {
                (true, _) | (_, true) => {
                    let mut pw = base_password.lock().unwrap();
                    let (overflow, _) = inc_carry_up(pw.len() - 1, pw.len() - 1, &mut pw);
                    if overflow {
                        pw.push(b' ');
                    }
                    pw.clone()
                }
                (false, false) => password,
            }
        }
        None => {
            let mut pw = base_password.lock().unwrap();
            if pw.is_empty() {
                if start_length == 0 {
                    pw.push(b' ');
                } else {
                    for _ in 0..start_length {
                        pw.push(b' ');
                    }
                }
            } else {
                let (overflow, _) = inc_carry_up(pw.len() - 1, pw.len() - 1, &mut pw);
                if overflow {
                    pw.push(b' ');
                }
            }
            pw.clone()
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

    #[test]
    fn next_password_generator_0() {
        let gen = next_password_generator(0);
        let ascii_list = ascii_list();

        for value in ascii_list.clone() {
            assert_eq!(gen(None), vec![value]);
        }
        assert_eq!(gen(None), vec![ascii_list[0], ascii_list[0]]);
        assert_eq!(gen(None), vec![ascii_list[0], ascii_list[1]]);
        assert_eq!(
            gen(Some(vec![ascii_list[0], ascii_list[1]])),
            vec![ascii_list[1], ascii_list[1]]
        );
        assert_eq!(gen(None), vec![ascii_list[0], ascii_list[2]]);
    }

    #[test]
    fn next_password_generator_1() {
        let gen = next_password_generator(1);
        let ascii_list = ascii_list();

        for value in ascii_list.clone() {
            assert_eq!(gen(None), vec![value]);
        }
        assert_eq!(gen(None), vec![ascii_list[0], ascii_list[0]]);
    }

    #[test]
    fn next_password_generator_2() {
        let gen = next_password_generator(2);
        let ascii_list = ascii_list();

        for value in ascii_list.clone() {
            assert_eq!(gen(None), vec![b' ', value]);
        }
        assert_eq!(gen(None), vec![ascii_list[0], ascii_list[0], ascii_list[0]]);
    }

    fn ascii_list() -> Vec<u8> {
        vec![
            b' ', b'!', b'"', b'#', b'$', b'%', b'&', b'\'', b'(', b')', b'*', b'+', b',', b'-',
            b'.', b'/', b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b':', b';',
            b'<', b'=', b'>', b'?', b'@', b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I',
            b'J', b'K', b'L', b'M', b'N', b'O', b'P', b'Q', b'R', b'S', b'T', b'U', b'V', b'W',
            b'X', b'Y', b'Z', b'[', b'\\', b']', b'^', b'_', b'`', b'a', b'b', b'c', b'd', b'e',
            b'f', b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n', b'o', b'p', b'q', b'r', b's',
            b't', b'u', b'v', b'w', b'x', b'y', b'z', b'{', b'|', b'}', b'~',
        ]
    }
}
