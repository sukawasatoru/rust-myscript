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

use std::fs;
use std::process::Command;
use tempfile::tempdir;

/// Counts every entry in `directory`, which the caller keeps private to one child process.
fn entry_count(directory: &std::path::Path) -> usize {
    fs::read_dir(directory).unwrap().count()
}

#[test]
fn running_the_server_does_not_leave_temporary_directories_behind() {
    let dir = tempdir().unwrap();
    let data = dir.path().join("data");
    // Point the child at a temporary directory of its own so the count cannot be disturbed by
    // other tests running in parallel.
    let temporary = dir.path().join("tmp");
    fs::create_dir(&temporary).unwrap();
    // Isolating libgit2 from the host configuration must not cost a directory that outlives
    // the process: the search path is blanked instead of pointed at a temporary directory.
    for _ in 0..3 {
        let output = Command::new(env!("CARGO_BIN_EXE_mcp-memo"))
            .arg(&data)
            .env("TMPDIR", &temporary)
            .env("TMP", &temporary)
            .env("TEMP", &temporary)
            .output()
            .unwrap();
        // The server exits once stdin closes; it must still have initialized the store.
        assert!(data.join(".git").is_dir(), "{output:?}");
        assert_eq!(entry_count(&temporary), 0);
    }
}

#[test]
fn legacy_startup_fails_and_migration_is_explicit_and_repeatable() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("memo.txt"), "legacy").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mcp-memo"))
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("migrate"));
    assert!(!dir.path().join(".git").exists());

    let output = Command::new(env!("CARGO_BIN_EXE_mcp-memo"))
        .arg(dir.path())
        .args(["migrate", "--dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stdout.is_empty());
    assert!(!dir.path().join(".git").exists());
    for expected in ["Migration completed", "Migration already completed"] {
        let output = Command::new(env!("CARGO_BIN_EXE_mcp-memo"))
            .arg(dir.path())
            .arg("migrate")
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
    }
    assert_eq!(
        fs::read_to_string(dir.path().join("memo.txt")).unwrap(),
        "legacy"
    );
}

#[test]
fn git_environment_and_invalid_external_config_do_not_affect_migration() {
    let dir = tempdir().unwrap();
    let data = dir.path().join("data");
    fs::create_dir(&data).unwrap();
    fs::write(data.join("memo.txt"), b"one\r\ntwo\r\n").unwrap();
    let xdg = dir.path().join("xdg");
    fs::create_dir_all(xdg.join("git")).unwrap();
    let config = xdg.join("git/config");
    fs::write(&config, "[invalid configuration\n").unwrap();
    let templates = dir.path().join("templates");
    fs::create_dir(&templates).unwrap();
    fs::write(templates.join("unexpected-template"), "must not copy").unwrap();
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mcp-memo"))
        .arg(&data)
        .arg("migrate")
        .env("XDG_CONFIG_HOME", xdg)
        .env("GIT_CONFIG_GLOBAL", &config)
        .env("GIT_CONFIG_SYSTEM", &config)
        .env("GIT_TEMPLATE_DIR", templates)
        .env("GIT_DIR", &outside)
        .env("GIT_WORK_TREE", &outside)
        .env("GIT_INDEX_FILE", outside.join("index"))
        .env("GIT_AUTHOR_NAME", "external author")
        .env("GIT_AUTHOR_EMAIL", "external@example.invalid")
        .env("GIT_COMMITTER_NAME", "external committer")
        .env("GIT_COMMITTER_EMAIL", "external@example.invalid")
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert!(output.stdout.is_empty());
    assert!(!data.join(".git/unexpected-template").exists());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    let repo = git2::Repository::open(&data).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(commit.author().name().unwrap(), "mcp-memo");
    assert_eq!(commit.committer().email().unwrap(), "mcp-memo@localhost");
    let tree = commit.tree().unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(
        repo.find_blob(tree.get_name("memo.txt").unwrap().id())
            .unwrap()
            .content(),
        b"one\r\ntwo\r\n"
    );
}

#[test]
fn external_config_found_through_home_does_not_reach_the_server() {
    let dir = tempdir().unwrap();
    let data = dir.path().join("data");
    // A host configuration discovered through the search path, not through GIT_CONFIG_*.
    // Blanking the search path is what keeps these out; nothing here is passed to libgit2
    // as an explicit config file.
    let home = dir.path().join("home");
    fs::create_dir_all(home.join("git")).unwrap();
    let attributes = home.join("attributes");
    fs::write(&attributes, "*.txt text eol=lf\n").unwrap();
    let templates = dir.path().join("templates");
    fs::create_dir(&templates).unwrap();
    fs::write(templates.join("unexpected-template"), "must not copy").unwrap();
    let external = format!(
        "[core]\n\tautocrlf = true\n\tattributesfile = {}\n[init]\n\ttemplateDir = {}\n\
         [user]\n\tname = external author\n\temail = external@example.invalid\n",
        attributes.display(),
        templates.display(),
    );
    for path in [home.join(".gitconfig"), home.join("git/config")] {
        fs::write(path, &external).unwrap();
    }

    // Seed a memo so the run records it, then let the process exit when stdin closes.
    fs::create_dir(&data).unwrap();
    fs::write(data.join("memo.txt"), b"one\r\ntwo\r\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mcp-memo"))
        .arg(&data)
        .arg("migrate")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", &home)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    // init.templateDir from the host config must not have been consulted.
    assert!(!data.join(".git/unexpected-template").exists());

    // Assert on what the child actually wrote rather than on this process's own Git config,
    // which is not isolated: the repository file must stay free of host settings, and the
    // committed bytes must survive `autocrlf` / `eol=lf` untouched.
    let local = fs::read_to_string(data.join(".git/config")).unwrap();
    for leaked in [
        "autocrlf",
        "attributesfile",
        "templateDir",
        "external author",
    ] {
        assert!(!local.contains(leaked), "{leaked} leaked into .git/config");
    }
    let repo = git2::Repository::open(&data).unwrap();
    let commit = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(commit.author().name().unwrap(), "mcp-memo");
    assert_eq!(commit.author().email().unwrap(), "mcp-memo@localhost");
    let tree = commit.tree().unwrap();
    assert_eq!(
        repo.find_blob(tree.get_name("memo.txt").unwrap().id())
            .unwrap()
            .content(),
        b"one\r\ntwo\r\n"
    );
}
