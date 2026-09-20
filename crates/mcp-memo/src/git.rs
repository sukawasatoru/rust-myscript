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

use anyhow::anyhow;
use git2::{ConfigLevel, Oid, Repository, RepositoryInitOptions, RepositoryOpenFlags, Signature};
use rust_myscript::prelude::*;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::Path;
use std::sync::OnceLock;

pub(crate) type Snapshot = BTreeMap<String, Vec<u8>>;

const FORMAT_FILE: &str = "mcp-memo-format";
const FORMAT_VERSION: &str = "1\n";
const EXCLUDES: &str = "/backup/\n/.mcp-memo.lock\n/.mcp-memo-*/\n";

fn configure_git() -> Fallible<()> {
    static CONFIG: OnceLock<Result<tempfile::TempDir, String>> = OnceLock::new();
    let result = CONFIG.get_or_init(|| {
        let configure = || -> Fallible<tempfile::TempDir> {
            let empty = tempfile::tempdir()?;
            for level in [ConfigLevel::System, ConfigLevel::Global, ConfigLevel::XDG] {
                // All application libgit2 calls pass through this OnceLock first.
                unsafe { git2::opts::set_search_path(level, empty.path())? };
            }
            #[cfg(windows)]
            unsafe {
                git2::opts::set_search_path(ConfigLevel::ProgramData, empty.path())?;
            }
            Ok(empty)
        };
        configure().map_err(|e| format!("{e:#}"))
    });
    result.as_ref().map(|_| ()).map_err(|e| anyhow!("{e}"))
}

pub(crate) fn validate_key(key: &str) -> Fallible<()> {
    if key.is_empty() {
        bail!("key must not be empty");
    }
    if !key
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        bail!("key must contain only alphanumeric characters, hyphens, underscores, or dots");
    }
    Ok(())
}

pub(crate) fn lock_directory(data_dir: &Path) -> Fallible<File> {
    let path = data_dir.join(".mcp-memo.lock");
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        ensure!(
            metadata.is_file(),
            "lock must be a regular file: {}",
            path.display()
        );
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock()?;
    Ok(file)
}

pub(crate) fn snapshot(data_dir: &Path) -> Fallible<Snapshot> {
    let mut snapshot = Snapshot::new();
    for entry in fs::read_dir(data_dir)? {
        let entry = entry?;
        if !entry.file_name().as_encoded_bytes().ends_with(b".txt") {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow!("memo filename is not UTF-8"))?;
        validate_key(name.strip_suffix(".txt").context("invalid memo filename")?)?;
        ensure!(
            entry.file_type()?.is_file(),
            "memo must be a regular file: {}",
            entry.path().display()
        );
        snapshot.insert(name, fs::read(entry.path())?);
    }
    Ok(snapshot)
}

pub(crate) fn init_repository(path: &Path) -> Fallible<Repository> {
    configure_git()?;
    let mut options = RepositoryInitOptions::new();
    options.external_template(false).initial_head("main");
    let repo = Repository::init_opts(path, &options)?;
    // Blob/tree writes below deliberately bypass attributes, filters and the user's index.
    repo.set_config(&git2::Config::new()?)?;
    fs::create_dir_all(repo.path().join("info"))?;
    fs::write(repo.path().join("info/exclude"), EXCLUDES)?;
    Ok(repo)
}

pub(crate) fn mark_ready(repo: &Repository) -> Fallible<()> {
    fs::write(repo.path().join(FORMAT_FILE), FORMAT_VERSION)?;
    Ok(())
}

pub(crate) fn open_repository(data_dir: &Path) -> Fallible<Option<Repository>> {
    configure_git()?;
    let path = data_dir.join(".git");
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(metadata) => ensure!(
            metadata.is_dir(),
            ".git must be a directory, not a link or file"
        ),
    }
    ensure!(
        fs::read_to_string(path.join(FORMAT_FILE)).ok().as_deref() == Some(FORMAT_VERSION),
        "existing .git is not a completed mcp-memo repository; refusing to modify it"
    );
    let repo = Repository::open_ext(
        &path,
        RepositoryOpenFlags::NO_SEARCH,
        std::iter::empty::<&Path>(),
    )?;
    repo.set_config(&git2::Config::new()?)?;
    ensure!(!repo.is_bare(), "expected a non-bare memo repository");
    ensure!(
        repo.head()?.name()? == "refs/heads/main",
        "memo repository HEAD must be on main"
    );
    repo.head()?.peel_to_commit()?;
    Ok(Some(repo))
}

pub(crate) fn ensure_repository(data_dir: &Path) -> Fallible<Repository> {
    if let Some(repo) = open_repository(data_dir)? {
        return Ok(repo);
    }
    ensure!(
        !data_dir.join("backup").try_exists()? && snapshot(data_dir)?.is_empty(),
        "legacy memos detected; stop old servers and run: mcp-memo '{}' migrate --dry-run, then migrate",
        data_dir.display()
    );
    let staging = tempfile::Builder::new()
        .prefix(".mcp-memo-init-")
        .tempdir_in(data_dir)?;
    let repo = init_repository(staging.path())?;
    commit_snapshot(
        &repo,
        &Snapshot::new(),
        "Initialize memo history",
        &signature_now()?,
        true,
    )?;
    mark_ready(&repo)?;
    drop(repo);
    fs::rename(staging.path().join(".git"), data_dir.join(".git"))?;
    open_repository(data_dir)?.context("failed to initialize memo repository")
}

pub(crate) fn signature_now() -> Fallible<Signature<'static>> {
    Ok(Signature::now("mcp-memo", "mcp-memo@localhost")?)
}

pub(crate) fn commit_snapshot(
    repo: &Repository,
    snapshot: &Snapshot,
    message: &str,
    signature: &Signature<'_>,
    force: bool,
) -> Fallible<Oid> {
    let mut builder = repo.treebuilder(None)?;
    for (name, content) in snapshot {
        validate_key(name.strip_suffix(".txt").context("invalid memo filename")?)?;
        builder.insert(name, repo.blob(content)?, 0o100644)?;
    }
    let tree = repo.find_tree(builder.write()?)?;
    let parent = match repo.head() {
        Ok(head) => Some(head.peel_to_commit()?),
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => None,
        Err(e) => return Err(e.into()),
    };
    if !force
        && let Some(parent) = &parent
        && parent.tree_id() == tree.id()
    {
        return Ok(parent.id());
    }
    let mut index = repo.index()?;
    index.read_tree(&tree)?;
    index.write()?;
    Ok(repo.commit(
        Some("HEAD"),
        signature,
        signature,
        message,
        &tree,
        &parent.iter().collect::<Vec<_>>(),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_preserve_bytes_and_only_track_memos() {
        let dir = tempfile::tempdir().unwrap();
        let repo = ensure_repository(dir.path()).unwrap();
        fs::write(dir.path().join("doc.txt"), b"one\r\ntwo\r\n").unwrap();
        fs::write(dir.path().join(".gitattributes"), "*.txt text eol=lf\n").unwrap();
        fs::write(dir.path().join("unrelated"), "do not track").unwrap();
        let state = snapshot(dir.path()).unwrap();
        let signature = signature_now().unwrap();
        let oid = commit_snapshot(&repo, &state, "Create memo", &signature, false).unwrap();
        let tree = repo.find_commit(oid).unwrap().tree().unwrap();
        assert_eq!(tree.len(), 1);
        let blob = repo
            .find_blob(tree.get_name("doc.txt").unwrap().id())
            .unwrap();
        assert_eq!(blob.content(), b"one\r\ntwo\r\n");
        assert_eq!(
            commit_snapshot(&repo, &state, "No change", &signature, false).unwrap(),
            oid
        );
    }

    #[test]
    fn startup_requires_migration_even_without_backups() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("doc.txt"), "legacy").unwrap();
        assert!(
            ensure_repository(dir.path())
                .err()
                .unwrap()
                .to_string()
                .contains("migrate")
        );
        assert!(!dir.path().join(".git").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("doc.txt")).unwrap(),
            "legacy"
        );
    }

    #[test]
    fn startup_rejects_unmanaged_repositories() {
        let dir = tempfile::tempdir().unwrap();
        init_repository(dir.path()).unwrap();
        assert!(
            ensure_repository(dir.path())
                .err()
                .unwrap()
                .to_string()
                .contains("refusing")
        );
    }
}
