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

//! Local dat storage. Callers receive parsed models; the mapper is an implementation detail.

mod dat_mapper;
use crate::model::{DatFileInfo, DownloadedDat, LoadedThread};
use rust_myscript::prelude::*;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Resolves a file specifier (thread number "630" or filename) to an actual path.
pub fn resolve_dat_file(dat_dir: &Path, file_spec: &str) -> Fallible<PathBuf> {
    let direct = dat_dir.join(file_spec);
    if direct.is_file() {
        return Ok(direct);
    }
    for entry in std::fs::read_dir(dat_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some((num, _)) = parse_dat_filename(&name_str)
            && num.to_string() == file_spec
        {
            return Ok(entry.path());
        }
    }
    bail!("dat file not found for: {file_spec}")
}

/// Returns all .dat file paths in dat_dir (sorted by filename).
pub fn list_all_dat_files(dat_dir: &Path) -> Fallible<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dat_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with(".dat")
            && parse_dat_filename(&name_str).is_some()
            && entry.path().is_file()
        {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

fn read_lines(path: &Path) -> Fallible<Vec<String>> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    Ok(reader.lines().collect::<Result<_, _>>()?)
}

pub fn load_thread(path: &Path) -> Fallible<LoadedThread> {
    let lines = read_lines(path)?;
    let file_info = build_file_info_from_lines(path, &lines)?;
    let posts = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| dat_mapper::parse_dat_line(line, index + 1))
        .collect();
    Ok(LoadedThread {
        file_info,
        posts,
        source_line_count: lines.len(),
        ref_counts: dat_mapper::count_references(&lines),
    })
}

pub fn build_file_info(path: &Path) -> Fallible<DatFileInfo> {
    build_file_info_from_lines(path, &read_lines(path)?)
}

/// Extracts (thread_num, thread_id) using the local `{prefix}_{num}_{id}.dat` convention.
pub fn parse_dat_filename(filename: &str) -> Option<(u32, String)> {
    let stem = filename.strip_suffix(".dat")?;
    let mut parts = stem.rsplitn(3, '_');
    let id = parts.next()?;
    let num_str = parts.next()?;
    parts.next()?;
    let num: u32 = num_str.parse().ok()?;
    Some((num, id.to_string()))
}

fn build_file_info_from_lines(path: &Path, lines: &[String]) -> Fallible<DatFileInfo> {
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let (thread_num, thread_id) = parse_dat_filename(&filename).unwrap_or((0, String::new()));
    let content = dat_mapper::parse_content_info(lines);
    Ok(DatFileInfo {
        filename,
        thread_num,
        thread_id,
        total_lines: content.total_lines,
        thread_title: content.thread_title,
        date_range: content.date_range,
    })
}

/// Resolves file specifiers to actual file paths. Returns all files if empty.
pub fn resolve_files(dat_dir: &Path, files: &[String]) -> Fallible<Vec<PathBuf>> {
    if files.is_empty() {
        return list_all_dat_files(dat_dir);
    }
    let mut result = Vec::new();
    for spec in files {
        result.push(resolve_dat_file(dat_dir, spec)?);
    }
    result.sort();
    Ok(result)
}

/// Preserve the existing behavior: an unreadable file has no previous count.
pub fn count_lines_if_exists(path: &Path) -> Option<usize> {
    let content = std::fs::read_to_string(path).ok()?;
    Some(content.lines().filter(|l| !l.is_empty()).count())
}

pub async fn save_dat(path: &Path, dat: &DownloadedDat) -> Fallible<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.with_context(|| {
            format!(
                "保存先ディレクトリの作成に失敗しました: {}",
                parent.display()
            )
        })?;
    }
    tokio::fs::write(path, dat.text().as_bytes())
        .await
        .with_context(|| format!("ファイルの書き込みに失敗しました: {}", path.display()))
}

#[cfg(test)]
pub mod test_helpers {
    use std::path::PathBuf;
    use tempfile::TempDir;

    pub struct TestDatDir {
        pub _dir: TempDir,
        pub dat_dir: PathBuf,
    }

    pub fn create_test_dat_dir() -> TestDatDir {
        let dir = TempDir::new().unwrap();
        let dat_dir = dir.path().join("dat_files");
        std::fs::create_dir_all(&dat_dir).unwrap();
        let dat_630 = [
            "テスト名<>sage<>2026/03/13(金) 10:38:56.82 ID:test0001<>最初のレス https://example.com/image001.jpg ここまで<>テストスレッド★630",
            "名無し<><>2026/03/13(金) 11:00:00.00 ID:test0002<>Tool v2.5すごい<br>&gt;&gt;1 これは便利<>",
            "名無し<>sage<>2026/03/13(金) 12:00:00.00 ID:test0003<>https://example.com/resources/12345 新しいプラグインが公開された<>",
            "名無し<><>2026/03/14(土) 09:00:00.00 ID:test0004<>App-X試してみた https://example.com/files/demo.mp4<>",
            "名無し<>sage<>2026/03/14(土) 10:00:00.00 ID:test0005<>https://example.com/repo/test Widget-Yも気になる<>",
        ].join("\n");
        std::fs::write(dat_dir.join("board_630_1773365936.dat"), &dat_630).unwrap();
        let dat_631 = [
            "テスト<>sage<>2026/03/18(水) 20:03:27.00 ID:test0010<>新スレ立てた<>テストスレッド★631",
            "名無し<><>2026/03/18(水) 21:00:00.00 ID:test0011<>Foobarで生成してみた https://example.com/output/xyz.png<>",
            "名無し<><>2026/03/18(水) 22:00:00.00 ID:test0012<>Bazqux<>",
        ].join("\n");
        std::fs::write(dat_dir.join("board_631_1773831807.dat"), &dat_631).unwrap();
        TestDatDir { _dir: dir, dat_dir }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn saved_dat_preserves_source_and_loads_display_models() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("board_700_123.dat");
        let source = "<b>名前</b><>sage<>2026/01/01 ID:a<>一行目<br>&gt;&gt;3<>タイトル\r\n\r\n名無し<><>2026/01/02 ID:b<>二番目<>\r\nOver 1000 Thread\r\n";
        let dat = DownloadedDat::from_text(source.to_owned());
        save_dat(&path, &dat).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(dat.res_count(), 3);

        let thread = load_thread(&path).unwrap();
        assert_eq!(thread.source_line_count, 4);
        assert_eq!(thread.file_info.total_lines, 2);
        assert_eq!(thread.posts.len(), 2);
        assert_eq!(thread.posts[0].res_num, 1);
        assert_eq!(thread.posts[0].name, "名前");
        assert_eq!(thread.posts[0].mail, "sage");
        assert_eq!(thread.posts[0].body, "一行目\n>>3");
        assert_eq!(thread.posts[1].res_num, 3);
        assert_eq!(thread.ref_counts.get(&3), Some(&1));
    }

    #[test]
    fn parse_dat_filename_normal() {
        let (num, id) = parse_dat_filename("board_630_1773365936.dat").unwrap();
        assert_eq!(num, 630);
        assert_eq!(id, "1773365936");
    }

    #[test]
    fn parse_dat_filename_invalid() {
        assert!(parse_dat_filename("invalid.dat").is_none());
    }

    #[test]
    fn resolve_dat_file_by_number() {
        let ctx = test_helpers::create_test_dat_dir();
        let path = resolve_dat_file(&ctx.dat_dir, "630").unwrap();
        assert!(path.to_string_lossy().contains("_630_"));
    }

    #[test]
    fn resolve_dat_file_by_name() {
        let ctx = test_helpers::create_test_dat_dir();
        let path = resolve_dat_file(&ctx.dat_dir, "board_630_1773365936.dat").unwrap();
        assert!(path.exists());
    }

    #[test]
    fn resolve_dat_file_not_found() {
        let ctx = test_helpers::create_test_dat_dir();
        assert!(resolve_dat_file(&ctx.dat_dir, "999").is_err());
    }

    #[test]
    fn list_all_dat_files_sorted() {
        let ctx = test_helpers::create_test_dat_dir();
        let files = list_all_dat_files(&ctx.dat_dir).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].to_string_lossy().contains("630"));
        assert!(files[1].to_string_lossy().contains("631"));
    }

    #[test]
    fn build_file_info_normal_thread() {
        let ctx = test_helpers::create_test_dat_dir();
        let info = build_file_info(&ctx.dat_dir.join("board_630_1773365936.dat")).unwrap();
        assert_eq!(info.total_lines, 5);
        assert_eq!(info.thread_num, 630);
        assert_eq!(info.thread_id, "1773365936");
        assert_eq!(info.thread_title, "テストスレッド★630");
        assert!(info.date_range.contains("2026/03/13"));
        assert!(info.date_range.contains("2026/03/14"));
    }
}
