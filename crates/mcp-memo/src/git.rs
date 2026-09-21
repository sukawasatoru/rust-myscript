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
    static CONFIG: OnceLock<Result<(), String>> = OnceLock::new();
    let result = CONFIG.get_or_init(|| {
        let configure = || -> Fallible<()> {
            // An empty search path is a list of zero directories, so libgit2 looks nowhere for
            // the user's config, attributes and ignore files. Repositories also get an empty
            // config of their own, so this is a second layer: it covers lookups that do not go
            // through a repository, such as the attribute and ignore files consulted via the
            // same search path. Blanking it costs no directory that has to be created and
            // cleaned up, so a process killed mid-write leaves nothing behind. Empty is a path
            // list rather than a relative path, so it never falls back to the working
            // directory, and it leaves no placeholder for anyone to drop a config file into.
            for level in [
                ConfigLevel::System,
                ConfigLevel::Global,
                ConfigLevel::XDG,
                ConfigLevel::ProgramData,
            ] {
                // All application libgit2 calls pass through this OnceLock first.
                unsafe { git2::opts::set_search_path(level, "")? };
            }
            Ok(())
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

/// Collects the memos, refusing to proceed when the directory holds anything unexpected.
///
/// Migration imports history once and irreversibly, so it must see the directory exactly as it
/// is; a stray file there may be a memo the user expects to be carried over. `snapshot` is the
/// lenient counterpart used on the write path.
pub(crate) fn strict_snapshot(data_dir: &Path) -> Fallible<Snapshot> {
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

pub(crate) fn snapshot(data_dir: &Path) -> Fallible<Snapshot> {
    let mut snapshot = Snapshot::new();
    for entry in fs::read_dir(data_dir)? {
        let entry = entry?;
        if !entry.file_name().as_encoded_bytes().ends_with(b".txt") {
            continue;
        }
        // Skip invalid names and non-regular files so stray entries do not block every write.
        // Memo changes are expected to go through MCP. If a tracked memo is manually replaced
        // with a symlink or directory, it is omitted here and recorded as deleted in the next
        // recovery commit; its previous content remains in earlier commits. Such manual
        // replacements are outside normal usage, so no special protection is provided.
        let Ok(name) = entry.file_name().into_string() else {
            warn!(
                path = %entry.path().display(),
                "ignoring memo file: filename is not UTF-8",
            );
            continue;
        };
        let Some(key) = name.strip_suffix(".txt") else {
            continue;
        };
        if let Err(e) = validate_key(key) {
            warn!(path = %entry.path().display(), %e, "ignoring memo file: invalid key");
            continue;
        }
        if !entry.file_type()?.is_file() {
            warn!(
                path = %entry.path().display(),
                "ignoring memo file: not a regular file",
            );
            continue;
        }
        // Read failures are different in kind: the name is valid, so this is a memo the store
        // owns and history must keep. Skipping it here would commit a tree without it, which
        // records the memo as deleted. Fail instead and leave the existing history intact.
        let content = fs::read(entry.path())
            .with_context(|| format!("failed to read memo file: {}", entry.path().display()))?;
        snapshot.insert(name, content);
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
        !data_dir.join("backup").try_exists()? && strict_snapshot(data_dir)?.is_empty(),
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

/// Writes `content` to the object database, skipping the write when it is already there.
///
/// `Repository::blob` recompresses the bytes on every call even when an identical blob exists,
/// which dominates the cost of committing a snapshot: nearly every memo is unchanged from the
/// previous commit, so almost all of that work is discarded. Hashing first and consulting the
/// odb turns those into a lookup. The resulting oid is the same either way, so the tree — and
/// therefore the commit — is byte-for-byte identical to what `blob` alone would have produced.
fn write_blob(repo: &Repository, odb: &git2::Odb<'_>, content: &[u8]) -> Fallible<Oid> {
    let oid = Oid::hash_object(git2::ObjectType::Blob, content)?;
    if odb.exists(oid) {
        return Ok(oid);
    }
    Ok(repo.blob(content)?)
}

pub(crate) fn commit_snapshot(
    repo: &Repository,
    snapshot: &Snapshot,
    message: &str,
    signature: &Signature<'_>,
    force: bool,
) -> Fallible<Oid> {
    let odb = repo.odb()?;
    let mut builder = repo.treebuilder(None)?;
    for (name, content) in snapshot {
        validate_key(name.strip_suffix(".txt").context("invalid memo filename")?)?;
        builder.insert(name, write_blob(repo, &odb, content)?, 0o100644)?;
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

/// Commits a single memo change on top of HEAD.
///
/// The caller must have just committed the directory's full snapshot (see `commit_snapshot`),
/// so HEAD's tree already matches what is on disk apart from this one entry. Building on that
/// tree instead of assembling a fresh one keeps the cost proportional to the change rather than
/// to the size of every memo, and yields exactly the tree a full rebuild would have produced.
///
/// `content` is the memo's new bytes, or `None` when it has been deleted.
pub(crate) fn commit_entry(
    repo: &Repository,
    name: &str,
    content: Option<&[u8]>,
    message: &str,
    signature: &Signature<'_>,
) -> Fallible<Oid> {
    validate_key(name.strip_suffix(".txt").context("invalid memo filename")?)?;
    let parent = repo.head()?.peel_to_commit()?;
    let mut builder = repo.treebuilder(Some(&parent.tree()?))?;
    match content {
        Some(content) => {
            let odb = repo.odb()?;
            builder.insert(name, write_blob(repo, &odb, content)?, 0o100644)?;
        }
        // A delete can arrive for an entry HEAD does not carry — the memo may have been
        // removed outside the server, or a previous delete committed but failed later on.
        // Treat that as nothing to do rather than an error.
        None if builder.get(name)?.is_some() => builder.remove(name)?,
        None => {}
    }
    let tree = repo.find_tree(builder.write()?)?;
    if parent.tree_id() == tree.id() {
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
        &[&parent],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_entry_matches_a_full_rebuild_for_creates_updates_and_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = ensure_repository(dir.path()).unwrap();
        let signature = signature_now().unwrap();
        // Seed a few memos so a full rebuild has something to differ over.
        let mut state = Snapshot::new();
        for (name, content) in [("a.txt", "one"), ("b.txt", "two"), ("c.txt", "three")] {
            fs::write(dir.path().join(name), content).unwrap();
            state.insert(name.to_owned(), content.as_bytes().to_vec());
        }
        commit_snapshot(&repo, &state, "Seed", &signature, false).unwrap();

        // Each change must produce the same tree whether built incrementally or from scratch.
        for (name, content) in [
            ("b.txt", Some(b"updated".as_slice())),
            ("d.txt", Some(b"created".as_slice())),
            ("a.txt", None),
        ] {
            match content {
                Some(content) => {
                    fs::write(dir.path().join(name), content).unwrap();
                    state.insert(name.to_owned(), content.to_vec());
                }
                None => {
                    fs::remove_file(dir.path().join(name)).unwrap();
                    state.remove(name);
                }
            }
            let oid = commit_entry(&repo, name, content, "Incremental", &signature).unwrap();
            let incremental = repo.find_commit(oid).unwrap().tree_id();
            let mut builder = repo.treebuilder(None).unwrap();
            for (name, content) in &state {
                builder
                    .insert(name, repo.blob(content).unwrap(), 0o100644)
                    .unwrap();
            }
            assert_eq!(incremental, builder.write().unwrap(), "{name}");
            assert_eq!(snapshot(dir.path()).unwrap(), state);
        }
    }

    #[test]
    fn commit_entry_skips_a_commit_when_the_content_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let repo = ensure_repository(dir.path()).unwrap();
        let signature = signature_now().unwrap();
        fs::write(dir.path().join("doc.txt"), b"same").unwrap();
        let first = commit_entry(&repo, "doc.txt", Some(b"same"), "Create", &signature).unwrap();
        let again = commit_entry(&repo, "doc.txt", Some(b"same"), "Again", &signature).unwrap();
        assert_eq!(first, again);
        // Deleting a memo that is not in the tree is likewise a no-op.
        assert_eq!(
            commit_entry(&repo, "missing.txt", None, "Delete", &signature).unwrap(),
            first
        );
    }

    #[test]
    fn write_blob_reuses_existing_objects_and_stores_new_ones() {
        let dir = tempfile::tempdir().unwrap();
        let repo = ensure_repository(dir.path()).unwrap();
        let odb = repo.odb().unwrap();
        let first = write_blob(&repo, &odb, b"content").unwrap();
        assert!(odb.exists(first));
        // The second call must return the same oid without needing a fresh write.
        assert_eq!(write_blob(&repo, &odb, b"content").unwrap(), first);
        assert_eq!(repo.find_blob(first).unwrap().content(), b"content");
        assert_ne!(write_blob(&repo, &odb, b"other").unwrap(), first);
    }

    #[test]
    fn snapshot_skips_foreign_entries_but_strict_snapshot_rejects_them() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("valid.txt"), b"keep").unwrap();
        fs::write(dir.path().join("my memo.txt"), b"invalid key").unwrap();
        fs::write(dir.path().join(".txt"), b"empty key").unwrap();
        fs::create_dir(dir.path().join("directory.txt")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("missing", dir.path().join("link.txt")).unwrap();

        assert_eq!(
            snapshot(dir.path()).unwrap(),
            Snapshot::from([("valid.txt".to_owned(), b"keep".to_vec())])
        );
        assert!(strict_snapshot(dir.path()).is_err());
        // Skipping must not touch the files themselves.
        assert_eq!(
            fs::read(dir.path().join("my memo.txt")).unwrap(),
            b"invalid key"
        );
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_fails_when_a_valid_memo_cannot_be_read() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("important.txt");
        fs::write(&path, b"precious").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let error = snapshot(dir.path()).unwrap_err();
        // Must name the offending file so the cause is actionable.
        assert!(format!("{error:#}").contains("important.txt"), "{error:#}");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            snapshot(dir.path()).unwrap(),
            Snapshot::from([("important.txt".to_owned(), b"precious".to_vec())])
        );
    }

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
