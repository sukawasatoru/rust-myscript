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

//! Command-line syntax for the binary.

use clap::{Parser, Subcommand};

/// Perplexity Search API MCP server
#[derive(Parser)]
#[command(arg_required_else_help = true)]
pub(crate) struct Opt {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Start the stdio MCP server
    Serve {
        /// Perplexity API key (overrides the environment and stored key)
        #[arg(long, env = "PERPLEXITY_API_KEY", hide_env_values = true)]
        api_key: Option<String>,
    },
    /// Manage the stored Perplexity API key
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum AuthCommand {
    /// Save or replace the API key (hidden terminal input by default)
    Set {
        /// Read one line from stdin instead of prompting
        #[arg(long)]
        stdin: bool,
    },
    /// Delete the saved API key
    Delete,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn struct_opt() {
        Opt::command().debug_assert();
    }

    #[test]
    fn auth_commands_and_serve_mode() {
        let err = Opt::try_parse_from(["mcp-pplx"]).err().unwrap();
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
        assert!(matches!(
            Opt::try_parse_from(["mcp-pplx", "serve"]).unwrap().command,
            Command::Serve { .. }
        ));
        for (args, expected_stdin) in [
            (vec!["mcp-pplx", "auth", "set"], false),
            (vec!["mcp-pplx", "auth", "set", "--stdin"], true),
        ] {
            let Command::Auth {
                command: AuthCommand::Set { stdin },
            } = Opt::try_parse_from(args).unwrap().command
            else {
                panic!("expected auth set")
            };
            assert_eq!(stdin, expected_stdin);
        }
        assert!(matches!(
            Opt::try_parse_from(["mcp-pplx", "auth", "delete"])
                .unwrap()
                .command,
            Command::Auth {
                command: AuthCommand::Delete
            }
        ));
        assert!(Opt::try_parse_from(["mcp-pplx", "--api-key", "test-key"]).is_err());
    }
}
