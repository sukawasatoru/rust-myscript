/*
 * Copyright 2024, 2026 sukawasatoru
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
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerInfo};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use rust_myscript::prelude::*;
use serde::Deserialize;
use tracing::Level;
use url::Url;

/// Create simple URL for amazon.
#[derive(Parser)]
struct Opt {
    /// Launch MCP Server.
    #[arg(long)]
    mcp: bool,

    /// Amazon URL.
    #[arg(value_hint = ValueHint::Url, required_unless_present = "mcp")]
    input: Option<Url>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_writer(std::io::stderr)
        .init();

    let opt = Opt::parse();

    match opt {
        Opt { mcp: true, .. } => {
            run_mcp_server().await;
        }
        Opt {
            input: Some(input), ..
        } => match create_short_url(&input) {
            Ok(short_url) => println!("{short_url}"),
            Err(_) => println!("{}", input),
        },
        Opt { input: None, .. } => {
            unreachable!("input is required unless --mcp is specified");
        }
    }
}

fn create_short_url(input: &Url) -> Fallible<Url> {
    let mut segments = match input.path_segments() {
        Some(data) => data,
        None => bail!("no base url"),
    };

    while let Some(segment) = segments.next() {
        if segment != "dp" {
            continue;
        }

        let id = match segments.next() {
            Some(data) => data,
            None => bail!("no dp id"),
        };

        let mut short_url = input.clone();
        short_url.set_path(&format!("/dp/{id}"));
        short_url.set_query(None);
        short_url.set_fragment(None);
        return Ok(short_url);
    }

    bail!("unsupported format")
}

struct McpServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl McpServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    /// Amazon の商品 URL を短縮する。商品名やクエリパラメータを除去し /dp/{ASIN} のみの URL を返す
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn amazon_dp(&self, params: Parameters<AmazonDpParams>) -> Result<String, String> {
        match Url::parse(&params.0.url) {
            Ok(url) => match create_short_url(&url) {
                Ok(short) => Ok(short.to_string()),
                Err(e) => {
                    warn!(?e, "unsupported format");
                    Err(e.to_string())
                }
            },
            Err(e) => {
                warn!(?e, "invalid url");
                Err(format!("invalid url: {e}"))
            }
        }
    }
}

#[tool_handler]
impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::default().with_server_info(Implementation::new(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        ))
    }
}

#[derive(Deserialize, JsonSchema)]
struct AmazonDpParams {
    /// Amazon の商品 URL
    url: String,
}

async fn run_mcp_server() {
    let server = McpServer::new().serve(rmcp::transport::stdio());
    let running = match server.await {
        Ok(running) => running,
        Err(e) => {
            error!(?e, "failed to initialize MCP server");
            return;
        }
    };

    if let Err(e) = running.waiting().await {
        error!(?e, "MCP server task panicked");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use rmcp::model::CallToolRequestParams;
    use rmcp::service::RunningService;
    use rmcp::{ClientHandler, RoleClient, ServiceExt};
    use serde_json::json;

    #[test]
    fn struct_opt() {
        Opt::command().debug_assert();
    }

    #[ignore]
    #[test]
    fn struct_opt_help() {
        Opt::command().print_help().unwrap();
    }

    #[test]
    fn struct_opt_url_format() {
        Opt::command().get_matches_from(["cmd-name", "https://example.com/bar"]);
    }

    #[test]
    fn struct_opt_data_uri_format() {
        Opt::command().get_matches_from(["cmd-name", "data:text/plain,HelloWorld"]);
    }

    #[test]
    fn create_short_url_data_uri() {
        let actual = create_short_url(&Url::parse("data:text/plain,HelloWorld").unwrap());
        assert!(actual.is_err());
    }

    #[test]
    fn create_short_url_empty_segments() {
        let actual = create_short_url(&Url::parse("https://www.amazon.co.jp").unwrap());
        assert!(actual.is_err());
    }

    #[test]
    fn create_short_url_short() {
        let actual =
            create_short_url(&Url::parse("https://www.amazon.co.jp/dp/12345").unwrap()).unwrap();
        assert_eq!(
            actual,
            Url::parse("https://www.amazon.co.jp/dp/12345").unwrap(),
        );
    }

    #[test]
    fn create_short_url_nintendo_switch() {
        let actual = create_short_url(&Url::parse("https://www.amazon.co.jp/Nintendo-Switch-Joy-ネオンブルー-ネオンレッド/dp/B0BM46DFH1/ref=sr_1_3?__mk_ja_JP=カタカナ&crid=2AQ07ADQ6ZENA&keywords=Nintendo+switch&qid=1705937708&sprefix=nintendo+switch%2Caps%2C183&sr=8-3").unwrap()).unwrap();
        assert_eq!(
            actual,
            Url::parse("https://www.amazon.co.jp/dp/B0BM46DFH1").unwrap(),
        );
    }

    #[test]
    fn create_short_url_nintendo_switch_with_fragment() {
        let actual = create_short_url(&Url::parse("https://www.amazon.co.jp/Nintendo-Switch-Joy-ネオンブルー-ネオンレッド/dp/B0BM46DFH1/ref=sr_1_3?__mk_ja_JP=カタカナ&crid=2AQ07ADQ6ZENA&keywords=Nintendo+switch&qid=1705937708&sprefix=nintendo+switch%2Caps%2C183&sr=8-3#detailBulletsWrapper_feature_div").unwrap()).unwrap();
        assert_eq!(
            actual,
            Url::parse("https://www.amazon.co.jp/dp/B0BM46DFH1").unwrap(),
        );
    }

    #[derive(Debug, Clone, Default)]
    struct DummyClientHandler;
    impl ClientHandler for DummyClientHandler {}

    struct McpTestContext {
        client: RunningService<RoleClient, DummyClientHandler>,
        server_handle: tokio::task::JoinHandle<anyhow::Result<()>>,
    }

    impl McpTestContext {
        async fn new() -> Fallible<Self> {
            let (server_transport, client_transport) = tokio::io::duplex(4096);
            let server_handle = tokio::spawn(async move {
                McpServer::new()
                    .serve(server_transport)
                    .await?
                    .waiting()
                    .await?;
                anyhow::Ok(())
            });
            let client = DummyClientHandler.serve(client_transport).await?;
            Ok(Self {
                client,
                server_handle,
            })
        }
    }

    impl Drop for McpTestContext {
        fn drop(&mut self) {
            self.server_handle.abort();
            self.client.cancellation_token().cancel();
        }
    }

    #[tokio::test]
    async fn mcp_amazon_dp_tool_short_url() -> Fallible<()> {
        let ctx = McpTestContext::new().await?;
        let result = ctx
            .client
            .call_tool(CallToolRequestParams::new("amazon_dp").with_arguments(
                json!({ "url": "https://www.amazon.co.jp/Nintendo-Switch/dp/B0BM46DFH1/ref=sr_1_3?keywords=Nintendo+switch" })
                    .as_object()
                    .unwrap()
                    .clone(),
            ))
            .await?;

        let text = result
            .content
            .first()
            .and_then(|c| c.raw.as_text())
            .map(|t| t.text.to_owned());

        assert_eq!(
            text.as_deref(),
            Some("https://www.amazon.co.jp/dp/B0BM46DFH1")
        );
        Ok(())
    }

    #[tokio::test]
    async fn mcp_amazon_dp_tool_invalid_url() -> Fallible<()> {
        let ctx = McpTestContext::new().await?;
        let result = ctx
            .client
            .call_tool(
                CallToolRequestParams::new("amazon_dp").with_arguments(
                    json!({ "url": "not-a-valid-url" })
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await?;

        assert!(result.is_error.unwrap_or(false), "expected error result");
        Ok(())
    }

    #[tokio::test]
    async fn mcp_amazon_dp_tool_no_dp_segment() -> Fallible<()> {
        let ctx = McpTestContext::new().await?;
        let result = ctx
            .client
            .call_tool(
                CallToolRequestParams::new("amazon_dp").with_arguments(
                    json!({ "url": "https://www.amazon.co.jp/s?k=Nintendo+switch" })
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await?;

        assert!(result.is_error.unwrap_or(false), "expected error result");
        Ok(())
    }
}
