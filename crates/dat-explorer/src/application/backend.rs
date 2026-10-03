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

use crate::application::thread_service::ThreadService;
use crate::data::mcp_client::McpClient;
use crate::model::viewer::{ThreadEntry, ThreadSnapshot};
use rust_myscript::prelude::*;
use std::sync::Arc;

/// Cloned when an operation starts, so switching does not change an in-flight request.
#[derive(Clone)]
pub enum Backend {
    Direct(Arc<ThreadService>),
    Mcp(Arc<McpClient>),
}

impl Backend {
    pub async fn list_threads(self) -> Fallible<Vec<ThreadEntry>> {
        match self {
            Self::Direct(service) => {
                tokio::task::spawn_blocking(move || service.list_threads()).await?
            }
            Self::Mcp(client) => client.list_threads().await,
        }
    }

    pub async fn load_thread(self, file: String) -> Fallible<ThreadSnapshot> {
        match self {
            Self::Direct(service) => {
                tokio::task::spawn_blocking(move || service.load_snapshot(&file)).await?
            }
            Self::Mcp(client) => client.load_thread(&file).await,
        }
    }
}
