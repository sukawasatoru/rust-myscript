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

use crate::git::{
    Snapshot, commit_snapshot, init_repository, lock_directory, mark_ready, open_repository,
    strict_snapshot, validate_key,
};
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use rust_myscript::prelude::*;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct MigrationReport {
    pub backup_count: usize,
    pub memo_count: usize,
    pub already_migrated: bool,
    pub dry_run: bool,
}

struct Backup {
    key: String,
    name: String,
    timestamp: DateTime<FixedOffset>,
    content: Vec<u8>,
}

pub async fn migrate(data_dir: PathBuf, dry_run: bool) -> Fallible<MigrationReport> {
    tokio::task::spawn_blocking(move || migrate_blocking(&data_dir, dry_run))
        .await
        .context("migration task failed")?
}

fn migrate_blocking(data_dir: &Path, dry_run: bool) -> Fallible<MigrationReport> {
    let _lock = lock_directory(data_dir)?;
    if let Some(repository) = open_repository(data_dir)? {
        return Ok(MigrationReport {
            backup_count: cleanup_backups(data_dir, &repository, dry_run).context(
                "Migration completed, but backup cleanup failed; rerun migrate to retry",
            )?,
            memo_count: 0,
            already_migrated: true,
            dry_run,
        });
    }

    let backups = read_backups(data_dir)?;
    let current = strict_snapshot(data_dir)?;
    let report = MigrationReport {
        backup_count: backups.len(),
        memo_count: current.len(),
        already_migrated: false,
        dry_run,
    };
    if dry_run {
        return Ok(report);
    }

    let staging = tempfile::Builder::new()
        .prefix(".mcp-memo-migration-")
        .tempdir_in(data_dir)
        .context("failed to create migration staging directory")?;
    let repository = init_repository(staging.path())?;
    let mut historical = Snapshot::new();
    for backup in backups {
        historical.insert(format!("{}.txt", backup.key), backup.content);
        let signature = migration_signature(&backup.timestamp)?;
        commit_snapshot(
            &repository,
            &historical,
            &format!("Migrate backup/{}/{}", backup.key, backup.name),
            &signature,
            true,
        )?;
    }

    let now = Utc::now().with_timezone(&FixedOffset::east_opt(9 * 60 * 60).unwrap());
    let signature = migration_signature(&now)?;
    commit_snapshot(
        &repository,
        &current,
        "Migrate current memos",
        &signature,
        true,
    )?;
    mark_ready(&repository)?;
    drop(repository);

    let destination = data_dir.join(".git");
    match fs::symlink_metadata(&destination) {
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("failed to inspect migration destination"),
        Ok(_) => bail!(
            "migration destination already exists: {}",
            destination.display()
        ),
    }
    fs::rename(staging.path().join(".git"), &destination)
        .context("failed to publish migrated repository")?;
    let repository = open_repository(data_dir)?.context("published repository is missing")?;
    cleanup_backups(data_dir, &repository, false)
        .context("Migration completed, but backup cleanup failed; rerun migrate to retry")?;
    Ok(report)
}

fn cleanup_backups(
    data_dir: &Path,
    repository: &git2::Repository,
    dry_run: bool,
) -> Fallible<usize> {
    let backup_dir = data_dir.join("backup");
    if !regular_directory_exists(&backup_dir)? {
        return Ok(0);
    }
    let mut imported = std::collections::BTreeMap::new();
    let mut walk = repository.revwalk()?;
    walk.push_head()?;
    for oid in walk {
        let commit = repository.find_commit(oid?)?;
        let Some(path) = commit.message()?.strip_prefix("Migrate backup/") else {
            continue;
        };
        let (key, name) = path
            .split_once('/')
            .context("invalid imported backup path")?;
        validate_key(key)?;
        ensure!(!matches!(key, "." | ".."), "invalid imported backup key");
        parse_backup_timestamp(name)?;
        let tree = commit.tree()?;
        let entry = tree
            .get_name(&format!("{key}.txt"))
            .context("imported backup is missing from history")?;
        imported.insert((key.to_owned(), name.to_owned()), entry.id());
    }
    let mut count = 0;
    let mut directories = std::collections::BTreeSet::new();
    for ((key, name), oid) in imported {
        let directory = backup_dir.join(key);
        if !regular_directory_exists(&directory)? {
            continue;
        }
        directories.insert(directory.clone());
        let path = directory.join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
            }
        };
        ensure!(
            metadata.is_file(),
            "backup must be a regular file: {}",
            path.display()
        );
        let content = fs::read(&path)?;
        ensure!(
            content == repository.find_blob(oid)?.content(),
            "backup differs from imported history; keeping {}",
            path.display()
        );
        if !dry_run {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove imported backup: {}", path.display()))?;
        }
        count += 1;
    }
    if !dry_run {
        for directory in directories {
            remove_empty_directory(&directory)?;
        }
        remove_empty_directory(&backup_dir)?;
    }
    Ok(count)
}

fn regular_directory_exists(path: &Path) -> Fallible<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_dir(),
                "backup must be a regular directory: {}",
                path.display()
            );
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

fn remove_empty_directory(path: &Path) -> Fallible<()> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove empty directory: {}", path.display())),
    }
}

fn migration_signature(timestamp: &DateTime<FixedOffset>) -> Fallible<git2::Signature<'static>> {
    Ok(git2::Signature::new(
        "mcp-memo",
        "mcp-memo@localhost",
        &git2::Time::new(
            timestamp.timestamp(),
            timestamp.offset().local_minus_utc() / 60,
        ),
    )?)
}

fn read_backups(data_dir: &Path) -> Fallible<Vec<Backup>> {
    let backup_dir = data_dir.join("backup");
    let metadata = match fs::symlink_metadata(&backup_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).context("failed to inspect backup directory"),
    };
    ensure!(metadata.is_dir(), "backup must be a regular directory");

    let mut backups = Vec::new();
    for entry in fs::read_dir(&backup_dir).context("failed to read backup directory")? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            continue;
        }
        ensure!(
            entry.file_type()?.is_dir(),
            "backup key must be a regular directory: {}",
            entry.path().display()
        );
        let key = backup_name_utf8(entry.file_name(), "backup key")?;
        validate_key(&key)?;
        for file in fs::read_dir(entry.path())
            .with_context(|| format!("failed to read backups for {key}"))?
        {
            let file = file?;
            if !file.file_name().as_encoded_bytes().ends_with(b".txt") {
                continue;
            }
            let path = file.path();
            ensure!(
                file.file_type()?.is_file(),
                "backup must be a regular file: {}",
                path.display()
            );
            let name = backup_name_utf8(file.file_name(), "backup filename")?;
            let timestamp = parse_backup_timestamp(&name)
                .with_context(|| format!("invalid backup filename: {}", path.display()))?;
            let content = fs::read(&path)
                .with_context(|| format!("failed to read backup: {}", path.display()))?;
            backups.push(Backup {
                key: key.clone(),
                name,
                timestamp,
                content,
            });
        }
    }
    backups.sort_by(|left, right| {
        left.timestamp
            .cmp(&right.timestamp)
            .then_with(|| left.key.cmp(&right.key))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(backups)
}

fn backup_name_utf8(name: std::ffi::OsString, kind: &str) -> Fallible<String> {
    name.into_string().map_err(|_| {
        std::io::Error::new(ErrorKind::InvalidData, format!("{kind} is not UTF-8")).into()
    })
}

fn parse_backup_timestamp(name: &str) -> Fallible<DateTime<FixedOffset>> {
    let stem = name
        .strip_suffix(".txt")
        .context("backup must end in .txt")?;
    let timestamp = NaiveDateTime::parse_from_str(stem, "%Y%m%d_%H%M%S_%9f")?;
    ensure!(
        timestamp.format("%Y%m%d_%H%M%S_%9f").to_string() == stem,
        "backup timestamp must use YYYYMMDD_HHMMSS_NNNNNNNNN"
    );
    FixedOffset::east_opt(9 * 60 * 60)
        .unwrap()
        .from_local_datetime(&timestamp)
        .single()
        .context("invalid backup timestamp")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    const FIRST: &str = "20260101_090001_000000001.txt";
    const SECOND: &str = "20260101_090001_000000002.txt";

    fn write_backup(data_dir: &Path, key: &str, name: &str, content: &[u8]) -> PathBuf {
        let directory = data_dir.join("backup").join(key);
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn history(repository: &git2::Repository) -> Vec<git2::Oid> {
        let mut commit = repository.head().unwrap().peel_to_commit().unwrap();
        let mut history = Vec::new();
        loop {
            history.push(commit.id());
            if commit.parent_count() == 0 {
                break;
            }
            assert_eq!(commit.parent_count(), 1);
            commit = commit.parent(0).unwrap();
        }
        history.reverse();
        history
    }

    fn committed_snapshot(repository: &git2::Repository, oid: git2::Oid) -> Snapshot {
        let tree = repository.find_commit(oid).unwrap().tree().unwrap();
        tree.iter()
            .map(|entry| {
                let blob = repository.find_blob(entry.id()).unwrap();
                (entry.name().unwrap().to_owned(), blob.content().to_vec())
            })
            .collect()
    }

    fn entries_without_lock(data_dir: &Path) -> Vec<std::ffi::OsString> {
        let mut entries: Vec<_> = fs::read_dir(data_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name != ".mcp-memo.lock")
            .collect();
        entries.sort();
        entries
    }

    #[tokio::test]
    async fn imports_in_filename_order_with_jst_dates_and_preserves_sources() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path();
        let old = "20251231_235959_999999999.txt";
        let backups = [
            (
                write_backup(data_dir, "zeta", SECOND, b"z2"),
                b"z2".to_vec(),
            ),
            (
                write_backup(data_dir, "alpha", SECOND, b"a1"),
                b"a1".to_vec(),
            ),
            (
                write_backup(data_dir, "gone", old, b"deleted"),
                b"deleted".to_vec(),
            ),
            (write_backup(data_dir, "zeta", FIRST, b"z1"), b"z1".to_vec()),
        ];
        for (index, (path, _)) in backups.iter().enumerate() {
            fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(
                    fs::FileTimes::new().set_modified(
                        SystemTime::UNIX_EPOCH + Duration::from_secs(index as u64 + 1),
                    ),
                )
                .unwrap();
        }
        fs::write(data_dir.join("alpha.txt"), b"current").unwrap();
        fs::write(data_dir.join("日本語.txt"), b"new").unwrap();
        let modified = fs::metadata(data_dir.join("alpha.txt"))
            .unwrap()
            .modified()
            .unwrap();
        let current = strict_snapshot(data_dir).unwrap();
        let before = Utc::now().timestamp();
        let report = migrate(data_dir.to_path_buf(), false).await.unwrap();
        let after = Utc::now().timestamp();
        assert_eq!(report.backup_count, 4);
        assert_eq!(report.memo_count, 2);
        assert!(!report.already_migrated);
        assert!(!report.dry_run);

        let repository = open_repository(data_dir).unwrap().unwrap();
        assert_eq!(
            fs::read_to_string(data_dir.join(".git/mcp-memo-format"))
                .unwrap()
                .trim(),
            "1"
        );
        let commits = history(&repository);
        assert_eq!(commits.len(), 5);
        let expected = [
            ("gone", old, b"deleted".as_slice(), 1767193199),
            ("zeta", FIRST, b"z1".as_slice(), 1767225601),
            ("alpha", SECOND, b"a1".as_slice(), 1767225601),
            ("zeta", SECOND, b"z2".as_slice(), 1767225601),
        ];
        let mut accumulated = Snapshot::new();
        for (oid, (key, name, content, seconds)) in commits.iter().zip(expected) {
            let commit = repository.find_commit(*oid).unwrap();
            assert!(
                commit
                    .message()
                    .unwrap()
                    .contains(&format!("backup/{key}/{name}"))
            );
            for signature in [commit.author(), commit.committer()] {
                assert_eq!(signature.when().seconds(), seconds);
                assert_eq!(signature.when().offset_minutes(), 540);
            }
            accumulated.insert(format!("{key}.txt"), content.to_vec());
            assert_eq!(committed_snapshot(&repository, *oid), accumulated);
        }
        let final_commit = repository.find_commit(*commits.last().unwrap()).unwrap();
        for signature in [final_commit.author(), final_commit.committer()] {
            assert!((before..=after).contains(&signature.when().seconds()));
        }
        assert_eq!(committed_snapshot(&repository, final_commit.id()), current);
        assert_eq!(strict_snapshot(data_dir).unwrap(), current);
        assert!(!data_dir.join("gone.txt").exists());
        assert!(!data_dir.join("zeta.txt").exists());
        for (path, _) in &backups {
            assert!(!path.exists());
        }
        assert!(!data_dir.join("backup").exists());
        assert_eq!(
            fs::metadata(data_dir.join("alpha.txt"))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
        assert!(
            !entries_without_lock(data_dir)
                .iter()
                .any(|name| name.to_string_lossy().starts_with(".mcp-memo-migration-"))
        );
    }

    #[test]
    fn duplicate_content_still_creates_one_commit_per_backup_and_a_final_commit() {
        let directory = tempfile::tempdir().unwrap();
        write_backup(directory.path(), "memo", FIRST, b"same");
        write_backup(directory.path(), "memo", SECOND, b"same");
        fs::write(directory.path().join("memo.txt"), b"same").unwrap();
        migrate_blocking(directory.path(), false).unwrap();
        let repository = open_repository(directory.path()).unwrap().unwrap();
        let commits = history(&repository);
        assert_eq!(commits.len(), 3);
        let trees: Vec<_> = commits
            .iter()
            .map(|oid| repository.find_commit(*oid).unwrap().tree_id())
            .collect();
        assert!(trees.iter().all(|tree| *tree == trees[0]));
    }

    #[test]
    fn migrates_current_only_and_empty_directories_with_or_without_backup_directory() {
        for has_backup_dir in [false, true] {
            for has_current in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                if has_backup_dir {
                    fs::create_dir(directory.path().join("backup")).unwrap();
                }
                if has_current {
                    fs::write(directory.path().join("memo.txt"), b"current").unwrap();
                }
                let expected = strict_snapshot(directory.path()).unwrap();
                let report = migrate_blocking(directory.path(), false).unwrap();
                assert_eq!(report.backup_count, 0);
                assert_eq!(report.memo_count, usize::from(has_current));
                let repository = open_repository(directory.path()).unwrap().unwrap();
                let commits = history(&repository);
                assert_eq!(commits.len(), 1);
                assert_eq!(committed_snapshot(&repository, commits[0]), expected);
            }
        }
    }

    #[test]
    fn backup_only_migration_finishes_with_an_empty_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        write_backup(directory.path(), "deleted", FIRST, b"old");
        let report = migrate_blocking(directory.path(), false).unwrap();
        assert_eq!(report.memo_count, 0);
        let repository = open_repository(directory.path()).unwrap().unwrap();
        let commits = history(&repository);
        assert_eq!(commits.len(), 2);
        assert!(committed_snapshot(&repository, commits[1]).is_empty());
        assert!(!directory.path().join("deleted.txt").exists());
    }

    #[test]
    fn dry_run_validates_and_counts_without_creating_history() {
        let directory = tempfile::tempdir().unwrap();
        let backup = write_backup(directory.path(), "memo", FIRST, b"old");
        fs::write(directory.path().join("memo.txt"), b"current").unwrap();
        let before = entries_without_lock(directory.path());
        let report = migrate_blocking(directory.path(), true).unwrap();
        assert_eq!(report.backup_count, 1);
        assert_eq!(report.memo_count, 1);
        assert!(!report.already_migrated);
        assert!(report.dry_run);
        assert_eq!(entries_without_lock(directory.path()), before);
        assert_eq!(fs::read(backup).unwrap(), b"old");
        assert_eq!(
            fs::read(directory.path().join("memo.txt")).unwrap(),
            b"current"
        );
        assert!(open_repository(directory.path()).unwrap().is_none());
    }

    #[test]
    fn ignores_ds_store_files_without_changing_sources() {
        for metadata_path in [
            ".DS_Store",
            "backup/.DS_Store",
            "backup/memo/.DS_Store",
            "backup/notes.txt",
            "backup/key",
            "backup/memo/notes.md",
            "backup/memo/.hidden",
            "backup/memo/draft.TXT",
            "backup/memo/20260101_090001_000000001.bak",
            "backup/memo/20260101_090001_000000001.txt.extra",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let data_dir = directory.path();
            let backup = write_backup(data_dir, "memo", FIRST, b"old");
            fs::write(data_dir.join("memo.txt"), b"current").unwrap();
            let metadata = data_dir.join(metadata_path);
            fs::write(&metadata, b"Finder metadata").unwrap();
            for dry_run in [true, false] {
                let report = migrate_blocking(data_dir, dry_run).unwrap();
                assert_eq!(report.backup_count, 1);
                assert_eq!(report.memo_count, 1);
                assert_eq!(report.dry_run, dry_run);
                assert_eq!(fs::read(&metadata).unwrap(), b"Finder metadata");
                if dry_run {
                    assert_eq!(fs::read(&backup).unwrap(), b"old");
                } else {
                    assert!(!backup.exists());
                }
                assert_eq!(fs::read(data_dir.join("memo.txt")).unwrap(), b"current");
                assert_eq!(data_dir.join(".git").exists(), !dry_run);
            }
            let repository = open_repository(data_dir).unwrap().unwrap();
            let commits = history(&repository);
            assert_eq!(commits.len(), 2);
            for (oid, content) in commits.into_iter().zip([b"old".as_slice(), b"current"]) {
                assert_eq!(
                    committed_snapshot(&repository, oid),
                    Snapshot::from([("memo.txt".to_owned(), content.to_vec())])
                );
            }
        }
    }

    #[test]
    fn cleanup_retry_preserves_unimported_files_and_history() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path();
        write_backup(data_dir, "a", FIRST, b"first");
        write_backup(data_dir, "b", FIRST, b"second");
        fs::write(data_dir.join("memo.txt"), b"current").unwrap();
        migrate_blocking(data_dir, false).unwrap();
        let repository = open_repository(data_dir).unwrap().unwrap();
        let before = history(&repository);
        let first = write_backup(data_dir, "a", FIRST, b"first");
        let second = write_backup(data_dir, "b", FIRST, b"changed after migration");
        let extra = write_backup(data_dir, "b", SECOND, b"not imported");
        let unknown = write_backup(data_dir, "b", "notes.md", b"keep");
        fs::create_dir(data_dir.join("backup/unrelated")).unwrap();

        let error = migrate_blocking(data_dir, false).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Migration completed, but backup cleanup failed")
        );
        assert!(format!("{error:#}").contains("differs from imported history"));
        assert!(!first.exists());
        assert_eq!(fs::read(&second).unwrap(), b"changed after migration");
        assert_eq!(history(&repository), before);
        assert_eq!(fs::read(data_dir.join("memo.txt")).unwrap(), b"current");

        fs::write(&second, b"second").unwrap();
        let preview = migrate_blocking(data_dir, true).unwrap();
        assert!(preview.already_migrated);
        assert_eq!(preview.backup_count, 1);
        assert_eq!(fs::read(&second).unwrap(), b"second");
        let report = migrate_blocking(data_dir, false).unwrap();
        assert!(report.already_migrated);
        assert_eq!(report.backup_count, 1);
        assert!(!second.exists());
        assert!(!data_dir.join("backup/a").exists());
        assert_eq!(fs::read(extra).unwrap(), b"not imported");
        assert_eq!(fs::read(unknown).unwrap(), b"keep");
        assert!(data_dir.join("backup/unrelated").is_dir());
        assert_eq!(migrate_blocking(data_dir, false).unwrap().backup_count, 0);
        assert_eq!(history(&repository), before);
    }

    #[test]
    fn cleanup_does_not_follow_links_or_remove_replacement_directories() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path();
        let backup = write_backup(data_dir, "memo", FIRST, b"old");
        migrate_blocking(data_dir, false).unwrap();
        fs::create_dir_all(&backup).unwrap();
        let extra = backup.join("keep.txt");
        fs::write(&extra, b"keep").unwrap();
        assert!(migrate_blocking(data_dir, false).is_err());
        assert_eq!(fs::read(&extra).unwrap(), b"keep");
        fs::remove_file(&extra).unwrap();
        fs::remove_dir(&backup).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = tempfile::tempdir().unwrap();
            let target = outside.path().join(FIRST);
            fs::write(&target, b"old").unwrap();
            symlink(&target, &backup).unwrap();
            assert!(migrate_blocking(data_dir, false).is_err());
            fs::remove_file(&backup).unwrap();
            fs::remove_dir(backup.parent().unwrap()).unwrap();
            symlink(outside.path(), backup.parent().unwrap()).unwrap();
            assert!(migrate_blocking(data_dir, false).is_err());
            fs::remove_file(backup.parent().unwrap()).unwrap();
            fs::remove_dir(data_dir.join("backup")).unwrap();
            symlink(outside.path(), data_dir.join("backup")).unwrap();
            assert!(migrate_blocking(data_dir, false).is_err());
            assert_eq!(fs::read(&target).unwrap(), b"old");
        }
    }

    #[test]
    fn failed_migration_keeps_all_backups() {
        let directory = tempfile::tempdir().unwrap();
        let backup = write_backup(directory.path(), "memo", FIRST, b"old");
        let invalid = write_backup(directory.path(), "memo", "notes.txt", b"invalid");
        for dry_run in [true, false] {
            assert!(migrate_blocking(directory.path(), dry_run).is_err());
            assert!(!directory.path().join(".git").exists());
            assert_eq!(fs::read(&backup).unwrap(), b"old");
            assert_eq!(fs::read(&invalid).unwrap(), b"invalid");
        }
    }

    #[test]
    fn keeps_unmanaged_entries_inside_backup_key_directory() {
        let directory = tempfile::tempdir().unwrap();
        let backup = write_backup(directory.path(), "memo", FIRST, b"old");
        let unknown = backup.parent().unwrap().join("unknown");
        fs::create_dir(&unknown).unwrap();
        fs::write(unknown.join("keep.txt"), b"keep").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("missing", backup.parent().unwrap().join("notes.link")).unwrap();
        migrate_blocking(directory.path(), false).unwrap();
        assert!(!backup.exists());
        assert_eq!(fs::read(unknown.join("keep.txt")).unwrap(), b"keep");
        #[cfg(unix)]
        assert!(
            fs::symlink_metadata(backup.parent().unwrap().join("notes.link"))
                .unwrap()
                .is_symlink()
        );
    }

    #[test]
    fn preserves_memos_named_ds_store() {
        let directory = tempfile::tempdir().unwrap();
        write_backup(directory.path(), ".DS_Store", FIRST, b"old memo");
        fs::write(directory.path().join(".DS_Store.txt"), b"current memo").unwrap();
        let report = migrate_blocking(directory.path(), false).unwrap();
        assert_eq!(report.backup_count, 1);
        assert_eq!(report.memo_count, 1);
        let repository = open_repository(directory.path()).unwrap().unwrap();
        let commits = history(&repository);
        assert_eq!(commits.len(), 2);
        assert_eq!(
            committed_snapshot(&repository, commits[0]),
            Snapshot::from([(".DS_Store.txt".to_owned(), b"old memo".to_vec())])
        );
        assert_eq!(
            committed_snapshot(&repository, commits[1]),
            Snapshot::from([(".DS_Store.txt".to_owned(), b"current memo".to_vec())])
        );
    }

    #[test]
    fn rejects_invalid_backup_names_including_noncanonical_dates() {
        for name in [
            "notes.txt",
            ".DS_Store.txt",
            "20260230_090001_000000001.txt",
            "20260101_250001_000000001.txt",
            "20260101_090001_1.txt",
            "20260101_090001_0000000001.txt",
        ] {
            for dry_run in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                write_backup(directory.path(), "valid", FIRST, b"valid");
                let invalid = write_backup(directory.path(), "invalid", name, b"invalid");
                let before = entries_without_lock(directory.path());
                assert!(
                    migrate_blocking(directory.path(), dry_run).is_err(),
                    "{name}"
                );
                assert_eq!(entries_without_lock(directory.path()), before);
                assert_eq!(fs::read(invalid).unwrap(), b"invalid");
                assert!(!directory.path().join(".git").exists());
            }
        }
    }

    #[test]
    fn rejects_invalid_keys_and_nonregular_content() {
        for dry_run in [false, true] {
            for invalid in [
                "backup/bad key",
                "backup/key/20260101_090001_000000001.txt",
                "memo.txt",
            ] {
                let directory = tempfile::tempdir().unwrap();
                fs::create_dir_all(directory.path().join(invalid)).unwrap();
                assert!(
                    migrate_blocking(directory.path(), dry_run).is_err(),
                    "{invalid}"
                );
                assert!(!directory.path().join(".git").exists());
            }
            for invalid in ["bad key.txt", ".txt", "backup"] {
                let directory = tempfile::tempdir().unwrap();
                let path = directory.path().join(invalid);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, b"invalid").unwrap();
                assert!(
                    migrate_blocking(directory.path(), dry_run).is_err(),
                    "{invalid}"
                );
                assert_eq!(fs::read(path).unwrap(), b"invalid");
                assert!(!directory.path().join(".git").exists());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_non_utf8_backup_names() {
        use std::os::unix::ffi::OsStringExt;
        use std::os::unix::fs::symlink;

        for dry_run in [false, true] {
            for invalid in [
                "memo.txt",
                "backup",
                "backup/key",
                "backup/key/20260101_090001_000000001.txt",
                "backup/.DS_Store",
            ] {
                for target in ["missing", "target"] {
                    let directory = tempfile::tempdir().unwrap();
                    fs::write(directory.path().join("target"), b"untouched").unwrap();
                    let path = directory.path().join(invalid);
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    symlink(directory.path().join(target), &path).unwrap();
                    assert!(
                        migrate_blocking(directory.path(), dry_run).is_err(),
                        "{invalid}"
                    );
                    assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
                    assert_eq!(
                        fs::read(directory.path().join("target")).unwrap(),
                        b"untouched"
                    );
                    assert!(!directory.path().join(".git").exists());
                }
            }
        }
        for (kind, valid) in [("backup key", "日本語"), ("backup filename", FIRST)] {
            assert_eq!(backup_name_utf8(valid.into(), kind).unwrap(), valid);
            let invalid = std::ffi::OsString::from_vec(vec![0xff]);
            let error = backup_name_utf8(invalid, kind).unwrap_err();
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().kind(),
                ErrorKind::InvalidData
            );
            assert_eq!(error.to_string(), format!("{kind} is not UTF-8"));
        }
    }

    #[test]
    fn repeat_migration_is_a_no_op() {
        let directory = tempfile::tempdir().unwrap();
        write_backup(directory.path(), "memo", FIRST, b"old");
        fs::write(directory.path().join("memo.txt"), b"current").unwrap();
        migrate_blocking(directory.path(), false).unwrap();
        let repository = open_repository(directory.path()).unwrap().unwrap();
        let before = history(&repository);
        drop(repository);
        for dry_run in [false, true] {
            let report = migrate_blocking(directory.path(), dry_run).unwrap();
            assert!(report.already_migrated);
            assert_eq!(report.dry_run, dry_run);
            let repository = open_repository(directory.path()).unwrap().unwrap();
            assert_eq!(history(&repository), before);
        }
    }

    #[test]
    fn rejects_unmanaged_and_invalid_git_without_overwriting_it() {
        for dry_run in [false, true] {
            for invalid in [false, true] {
                let directory = tempfile::tempdir().unwrap();
                if invalid {
                    fs::write(directory.path().join(".git"), b"invalid repository").unwrap();
                } else {
                    drop(init_repository(directory.path()).unwrap());
                }
                fs::write(directory.path().join("memo.txt"), b"current").unwrap();
                assert!(migrate_blocking(directory.path(), dry_run).is_err());
                assert_eq!(
                    fs::read(directory.path().join("memo.txt")).unwrap(),
                    b"current"
                );
                if invalid {
                    assert_eq!(
                        fs::read(directory.path().join(".git")).unwrap(),
                        b"invalid repository"
                    );
                } else {
                    assert!(!directory.path().join(".git/mcp-memo-format").exists());
                }
            }
        }
    }

    #[test]
    fn interrupted_staging_repository_is_not_activated() {
        for ready in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let staging = tempfile::Builder::new()
                .prefix(".mcp-memo-migration-")
                .tempdir_in(directory.path())
                .unwrap();
            let repository = init_repository(staging.path()).unwrap();
            if ready {
                mark_ready(&repository).unwrap();
            }
            drop(repository);
            assert!(open_repository(directory.path()).unwrap().is_none());
            fs::write(directory.path().join("memo.txt"), b"current").unwrap();
            migrate_blocking(directory.path(), false).unwrap();
            let repository = open_repository(directory.path()).unwrap().unwrap();
            assert_eq!(history(&repository).len(), 1);
            assert!(staging.path().join(".git").exists());
            assert_eq!(staging.path().join(".git/mcp-memo-format").exists(), ready);
        }
    }

    #[test]
    fn rejects_an_invalid_completion_marker() {
        for dry_run in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            drop(init_repository(directory.path()).unwrap());
            let marker = directory.path().join(".git/mcp-memo-format");
            fs::write(&marker, b"unsupported-format").unwrap();
            assert!(migrate_blocking(directory.path(), dry_run).is_err());
            assert_eq!(fs::read(marker).unwrap(), b"unsupported-format");
        }
    }
}
