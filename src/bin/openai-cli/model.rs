/*
 * Copyright 2023 sukawasatoru
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

pub use crate::model::file_version::FileVersion;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod file_version;

#[derive(Deserialize, Serialize)]
pub struct Settings {
    pub organization_id: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatID(pub Uuid);

#[derive(Debug, Eq, PartialEq)]
pub struct Chat {
    pub chat_id: ChatID,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub model_id: String,
}

#[derive(Debug, Eq, PartialEq)]
pub struct MessageID(pub Uuid);

#[derive(Debug, Eq, PartialEq)]
pub struct Message {
    pub message_id: MessageID,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub role: MessageRole,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}
