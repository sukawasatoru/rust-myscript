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

//! # mcp-pplx
//!
//! A stdio MCP server for the Perplexity Search API.
//!
//! ## Registering an API key
//!
//! Register the key once, using the same OS and user account that will run the
//! MCP server. Input is hidden. Run the same command to replace an existing key.
//!
//! ```sh
//! mcp-pplx auth set
//! ```
//!
//! Start the server with the `serve` subcommand. Set the MCP client's `command`
//! to `mcp-pplx` and its `args` to `["serve"]`. Running without a subcommand
//! displays help instead of starting the server.
//!
//! ```sh
//! mcp-pplx serve
//! ```
//!
//! API key precedence is `serve --api-key` > `PERPLEXITY_API_KEY` > the stored key.
//! An option or environment override bypasses credential storage entirely and is
//! not saved. An empty or invalid override is an error, not a request to fall back.
//! If no override or stored key is available, the server prints registration
//! instructions to stderr and exits with a nonzero status.
//!
//! To store an existing environment variable without displaying the key, pipe it
//! to `auth set --stdin`. This reads the first line and removes only its trailing
//! LF or CRLF.
//!
//! ```sh
//! # bash / zsh
//! printf '%s\n' "$PERPLEXITY_API_KEY" | mcp-pplx auth set --stdin
//! unset PERPLEXITY_API_KEY
//! ```
//!
//! ```powershell
//! $env:PERPLEXITY_API_KEY | mcp-pplx auth set --stdin
//! Remove-Item Env:PERPLEXITY_API_KEY
//! ```
//!
//! After migration, also remove keys from shell startup files and MCP client
//! configuration. Running servers retain the key in memory, so restart them after
//! updating or deleting the stored key.
//!
//! ```sh
//! mcp-pplx auth delete
//! ```
//!
//! ## Storage
//!
//! `keyring-core` provides the common API. The backend is selected for the build's
//! target OS. The service is `rust-myscript.mcp-pplx` and the user is
//! `perplexity-api-key`.
//!
//! | Platform | Backend | Storage location |
//! | --- | --- | --- |
//! | Windows | `windows-native-keyring-store` | Windows Credential Manager (generic credentials) |
//! | macOS | `apple-native-keyring-store`, `keychain` feature | macOS Keychain |
//! | Linux / WSL (Ubuntu 24 or later) | `db-keystore` | `$XDG_STATE_HOME/mcp-pplx/credentials.db`, or `~/.local/state/mcp-pplx/credentials.db` if unset |
//!
//! The Linux directory is obtained by constructing
//! `directories::ProjectDirs::from("com", "sukawasatoru", "mcp-pplx")`
//! and calling `state_dir()` on the resulting `ProjectDirs`.
//! A dedicated database keeps storage settings, backups, and deletion independent
//! of other applications using `db-keystore`.
//!
//! On Linux, SQLite stores the key **in plaintext, without encryption**. The
//! dedicated directory has mode `0700`, restricting access to the database and
//! auxiliary files such as the WAL to its owner. On WSL, keep this directory on
//! the Linux filesystem, rather than moving it to `/mnt/c` or a similar mount.
//! The Linux backend does not use Secret Service, D-Bus, or Windows Credential
//! Manager. Running the Windows executable from WSL uses Windows storage instead.
//!
//! `db-keystore` uses Turso rather than the OS SQLite library. Its I/O backend is
//! explicitly set to `syscall`, so `io_uring` support is not required.
//!
//! To build from source on Ubuntu, install C/C++ build tools in addition to Rust:
//!
//! ```sh
//! sudo apt install build-essential pkg-config cmake libclang-dev
//! cargo build --release --locked -p mcp-pplx
//! ```
//!
//! References: [keyring-core](https://docs.rs/keyring-core/1.0.1/keyring_core/),
//! [Windows](https://docs.rs/windows-native-keyring-store/1.1.0/windows_native_keyring_store/),
//! [macOS](https://docs.rs/apple-native-keyring-store/1.0.2/apple_native_keyring_store/),
//! [db-keystore](https://docs.rs/db-keystore/0.6.0/db_keystore/).
//!
//! ## Modules
//!
//! [`feature`] exposes authentication commands and the MCP server. Credential
//! storage, HTTP access, and wire types remain internal to this library.
//! CLI parsing and blocking-task orchestration belong to the binary.

mod data;
mod dto;
pub mod feature;
