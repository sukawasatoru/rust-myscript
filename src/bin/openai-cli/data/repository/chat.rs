/*
 * Copyright 2023, 2024, 2025 sukawasatoru
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

use crate::model::{Chat, ChatID, FileVersion, Message, MessageID, MessageRole};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
use rusqlite::{Connection, ToSql, named_params, params_from_iter};
use rust_myscript::model::SQLiteUserVersion;
use rust_myscript::prelude::*;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use tinytable_rs::Attribute::{NOT_NULL, PRIMARY_KEY};
use tinytable_rs::ForeignKeyAttribute::{
    CASCADE, DEFERRABLE_INITIALLY_DEFERRED, ON_DELETE, REFERENCES,
};
use tinytable_rs::Type::{INTEGER, TEXT};
use tinytable_rs::{Column, Table, column, foreign_key};
use tracing::instrument;
use uuid::Uuid;

pub trait GetChatRepository {
    type Repo: ChatRepository;

    fn get_chat_repo(&self) -> &Self::Repo;
}

pub trait ChatRepository {
    fn find_chat_all(&self) -> Fallible<Vec<(Chat, Vec<Message>)>>;

    #[allow(dead_code)]
    fn find_chat(&self, id: &ChatID) -> Fallible<Option<(Chat, Vec<Message>)>>;

    fn save_chat(&self, chat: &Chat) -> Fallible<()>;

    fn save_messages(&self, chat_id: &ChatID, messages: &[Message]) -> Fallible<()>;

    #[allow(dead_code)]
    fn delete_messages(&self, message_ids: &[MessageID]) -> Fallible<usize>;
}

pub struct ChatRepositoryImpl {
    conn: Mutex<Connection>,
    chat_table: ChatTable,
    message_table: MessageTable,
}

impl ChatRepositoryImpl {
    pub fn create_with_path(version: &FileVersion, db_dir_path: &Path) -> Fallible<Self> {
        if !db_dir_path.exists() {
            fs::create_dir_all(db_dir_path)?;
        }

        Self::create_with_conn(version, Connection::open(db_dir_path.join("chat.db"))?)
    }

    #[instrument(skip(conn))]
    fn create_with_conn(version: &FileVersion, mut conn: Connection) -> Fallible<Self> {
        let chat_table = ChatTable::default();
        let message_table = MessageTable::create(&chat_table);

        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;

        let transaction = conn.transaction()?;
        let user_version = Self::retrieve_user_version(&transaction)?;

        if user_version == SQLiteUserVersion::from(0) {
            info!("initialize db");

            transaction.execute(&chat_table.create_sql(), [])?;
            transaction.execute(&message_table.create_sql(), [])?;
            for index in chat_table.create_index() {
                transaction.execute(&index, [])?;
            }

            for index in message_table.create_index() {
                transaction.execute(&index, [])?;
            }
        } else {
            #[allow(clippy::collapsible_else_if)]
            if user_version < "0.1.1".parse()? {
                info!("migrate to v0.1.1");

                transaction.execute(
                    "CREATE INDEX index_chat_created_at ON chat (created_at)",
                    [],
                )?;
                transaction.execute(
                    "CREATE INDEX index_message_created_at ON message (created_at)",
                    [],
                )?;
            }
        }

        Self::save_user_version(
            &transaction,
            &SQLiteUserVersion::from((
                version.major.try_into()?,
                version.minor.try_into()?,
                version.patch.try_into()?,
            )),
        )?;

        if version != &FileVersion::from(Self::retrieve_user_version(&transaction)?) {
            bail!("need to update file version: {}", version);
        }

        transaction.commit()?;

        conn.execute("PRAGMA foreign_keys = ON", [])?;

        Ok(Self {
            conn: Mutex::new(conn),
            chat_table,
            message_table,
        })
    }

    fn get_conn(&self) -> Fallible<MutexGuard<'_, Connection>> {
        match self.conn.lock() {
            Ok(data) => Ok(data),
            Err(_) => bail!("failed to get connection"),
        }
    }

    fn retrieve_user_version(conn: &Connection) -> Fallible<SQLiteUserVersion> {
        Ok(conn
            .prepare_cached("pragma user_version")?
            .query([])?
            .next()?
            .context("failed to query the user_version")?
            .get(0)?)
    }

    fn save_user_version(
        conn: &Connection,
        version: &SQLiteUserVersion,
    ) -> rusqlite::Result<usize> {
        conn.execute(
            &format!("PRAGMA user_version = {}", u32::from(version) as i32),
            [],
        )
    }
}

impl ChatRepository for ChatRepositoryImpl {
    fn find_chat_all(&self) -> Fallible<Vec<(Chat, Vec<Message>)>> {
        let mut conn = self.get_conn()?;
        let transaction = conn.transaction()?;

        let mut ret = vec![];

        let chats = {
            let mut statement = transaction.prepare_cached(&format!(
                "select * from {table} order by {created_at}",
                table = self.chat_table.name(),
                created_at = self.chat_table.created_at.name(),
            ))?;
            let index_chat_id = statement.column_index(self.chat_table.chat_id.name())?;
            let index_title = statement.column_index(self.chat_table.title.name())?;
            let index_created_at = statement.column_index(self.chat_table.created_at.name())?;
            let index_model_id = statement.column_index(self.chat_table.model_id.name())?;

            // for statement.
            #[allow(clippy::let_and_return)]
            let chats = statement
                .query([])?
                .mapped(|row| {
                    Ok(Chat {
                        chat_id: row.get(index_chat_id)?,
                        title: row.get(index_title)?,
                        created_at: row.get::<_, RepoDateTime>(index_created_at)?.0,
                        model_id: row.get(index_model_id)?,
                    })
                })
                .collect::<Result<Vec<Chat>, _>>()
                .context("map chat rows")?;
            chats
        };

        let mut statement = transaction.prepare_cached(&format!(
            "select * from {table} where {chat_id} = :chat_id order by {created_at}",
            table = self.message_table.name(),
            chat_id = self.message_table.chat_id.name(),
            created_at = self.message_table.created_at.name(),
        ))?;
        let index_message_id = statement.column_index(self.message_table.message_id.name())?;
        let index_created_at = statement.column_index(self.message_table.created_at.name())?;
        let index_updated_at = statement.column_index(self.message_table.updated_at.name())?;
        let index_role = statement.column_index(self.message_table.role.name())?;
        let index_text = statement.column_index(self.message_table.text.name())?;
        for chat in chats {
            let messages = statement
                .query(named_params! {":chat_id": chat.chat_id})?
                .mapped(|row| {
                    Ok(Message {
                        message_id: row.get(index_message_id)?,
                        created_at: row.get::<_, RepoDateTime>(index_created_at)?.0,
                        updated_at: row.get::<_, RepoDateTime>(index_updated_at)?.0,
                        role: row.get(index_role)?,
                        text: row.get(index_text)?,
                    })
                })
                .collect::<Result<Vec<Message>, _>>()
                .context("map message rows")?;
            ret.push((chat, messages));
        }
        drop(statement);

        transaction.commit()?;

        Ok(ret)
    }

    fn find_chat(&self, id: &ChatID) -> Fallible<Option<(Chat, Vec<Message>)>> {
        let mut conn = self.get_conn()?;
        let transaction = conn.transaction()?;

        let mut statement = transaction.prepare_cached(&format!(
            "select * from {table} where {chat_id} = :chat_id order by {created_at}",
            table = self.chat_table.name(),
            chat_id = self.chat_table.chat_id.name(),
            created_at = self.chat_table.created_at.name(),
        ))?;
        let chat = statement
            .query(named_params! {":chat_id": id})?
            .mapped(|row| {
                Ok(Chat {
                    chat_id: row.get(self.chat_table.chat_id.name())?,
                    title: row.get(self.chat_table.title.name())?,
                    created_at: row
                        .get::<_, RepoDateTime>(self.chat_table.created_at.name())?
                        .0,
                    model_id: row.get(self.chat_table.model_id.name())?,
                })
            })
            .next();
        let chat = match chat {
            Some(Ok(data)) => data,
            Some(Err(e)) => return Err(e.into()),
            None => {
                debug!("not found");
                return Ok(None);
            }
        };
        drop(statement);

        let mut statement = transaction.prepare_cached(&format!(
            "select * from {table} where {chat_id} = :chat_id order by {created_at}",
            table = self.message_table.name(),
            chat_id = self.message_table.chat_id.name(),
            created_at = self.message_table.created_at.name(),
        ))?;
        let index_message_id = statement.column_index(self.message_table.message_id.name())?;
        let index_created_at = statement.column_index(self.message_table.created_at.name())?;
        let index_updated_at = statement.column_index(self.message_table.updated_at.name())?;
        let index_role = statement.column_index(self.message_table.role.name())?;
        let index_text = statement.column_index(self.message_table.text.name())?;
        let messages = statement
            .query(named_params! {":chat_id": chat.chat_id})?
            .mapped(|row| {
                Ok(Message {
                    message_id: row.get(index_message_id)?,
                    created_at: row.get::<_, RepoDateTime>(index_created_at)?.0,
                    updated_at: row.get::<_, RepoDateTime>(index_updated_at)?.0,
                    role: row.get(index_role)?,
                    text: row.get(index_text)?,
                })
            })
            .collect::<Result<Vec<Message>, _>>()
            .context("map message rows")?;
        drop(statement);

        transaction.commit()?;

        Ok(Some((chat, messages)))
    }

    fn save_chat(&self, chat: &Chat) -> Fallible<()> {
        let mut conn = self.get_conn()?;
        let transaction = conn.transaction()?;

        let mut statement = transaction.prepare_cached(&format!("insert into {table}({key_chat_id}, {key_title}, {key_created_at}, {key_model_id}) values(:chat_id, :title, :created_at, :model_id) on conflict ({key_chat_id}) do update set ({key_title}, {key_created_at}, {key_model_id}) = (excluded.{key_title}, excluded.{key_created_at}, excluded.{key_model_id})",
            table = self.chat_table.name(),
            key_chat_id = self.chat_table.chat_id.name(),
            key_title = self.chat_table.title.name(),
            key_created_at = self.chat_table.created_at.name(),
            key_model_id = self.chat_table.model_id.name(),
        ))?;
        statement.insert(named_params! {
            ":chat_id": chat.chat_id,
            ":title": chat.title,
            ":created_at": RepoDateTimeRef(&chat.created_at),
            ":model_id": chat.model_id,
        })?;
        drop(statement);
        transaction.commit()?;
        Ok(())
    }

    fn save_messages(&self, chat_id: &ChatID, messages: &[Message]) -> Fallible<()> {
        let mut conn = self.get_conn()?;
        let transaction = conn.transaction()?;
        let mut statement = transaction.prepare_cached(&format!(
            "insert into {table}({key_message_id}, {key_chat_id}, {key_created_at}, {key_updated_at}, {key_role}, {key_text}) values (:message_id, :chat_id, :created_at, :updated_at, :role, :text) on conflict ({key_message_id}) do update set ({key_chat_id}, {key_created_at}, {key_updated_at}, {key_role}, {key_text}) = (:chat_id, :created_at, :updated_at, :role, :text)",
            table = self.message_table.name(),
            key_message_id = self.message_table.message_id.name(),
            key_chat_id = self.message_table.chat_id.name(),
            key_created_at = self.message_table.created_at.name(),
            key_updated_at = self.message_table.updated_at.name(),
            key_role = self.message_table.role.name(),
            key_text = self.message_table.text.name(),
        ))?;

        for message in messages {
            statement.insert(named_params! {
                ":message_id": message.message_id,
                ":chat_id": chat_id,
                ":created_at": RepoDateTimeRef(&message.created_at),
                ":updated_at": RepoDateTimeRef(&message.updated_at),
                ":role": message.role,
                ":text": message.text,
            })?;
        }

        drop(statement);
        transaction.commit()?;
        Ok(())
    }

    fn delete_messages(&self, message_ids: &[MessageID]) -> Fallible<usize> {
        let mut conn = self.get_conn()?;
        let transaction = conn.transaction()?;
        let mut statement = transaction.prepare_cached(&format!(
            "delete from {table} where {key_message_id} in ({key_ids})",
            table = self.message_table.name(),
            key_message_id = self.message_table.message_id.name(),
            key_ids = message_ids
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(","),
        ))?;

        let affect = statement.execute(params_from_iter(message_ids))?;

        drop(statement);
        transaction.commit()?;

        Ok(affect)
    }
}

impl Drop for ChatRepositoryImpl {
    #[instrument(skip_all)]
    fn drop(&mut self) {
        debug!("drop chat repo impl");

        let conn = match self.get_conn() {
            Ok(data) => data,
            Err(e) => {
                warn!(?e);
                return;
            }
        };

        match conn.query_row("pragma analysis_limit=400", [], |_| Ok(())) {
            Ok(_) => {}
            Err(e) => {
                warn!(?e, "pragma analysis_limit");
                return;
            }
        }

        match conn.execute("pragma optimize", []) {
            Ok(_) => {}
            Err(e) => warn!(?e, "pragma optimize"),
        }
    }
}

struct ChatTable {
    chat_id: Arc<Column>,
    title: Arc<Column>,
    created_at: Arc<Column>,
    model_id: Arc<Column>,
    columns: Vec<Arc<Column>>,
    indexes: Vec<(String, Vec<Arc<Column>>)>,
}

impl Default for ChatTable {
    fn default() -> Self {
        let chat_id = column("chat_id", TEXT, [PRIMARY_KEY, NOT_NULL]);
        let title = column("title", TEXT, [NOT_NULL]);
        let created_at = column("created_at", INTEGER, [NOT_NULL]);
        let model_id = column("model_id", TEXT, [NOT_NULL]);
        Self {
            chat_id: chat_id.clone(),
            title: title.clone(),
            created_at: created_at.clone(),
            model_id: model_id.clone(),
            columns: vec![chat_id, title, created_at.clone(), model_id],
            indexes: vec![("index_chat_created_at".into(), vec![created_at])],
        }
    }
}

impl Table for ChatTable {
    fn name(&self) -> &str {
        "chat"
    }

    fn columns(&self) -> &[Arc<Column>] {
        &self.columns
    }

    fn indexes(&self) -> &[(String, Vec<Arc<Column>>)] {
        &self.indexes
    }
}

struct MessageTable {
    message_id: Arc<Column>,
    chat_id: Arc<Column>,
    created_at: Arc<Column>,
    updated_at: Arc<Column>,
    role: Arc<Column>,
    text: Arc<Column>,
    columns: Vec<Arc<Column>>,
    indexes: Vec<(String, Vec<Arc<Column>>)>,
}

impl MessageTable {
    pub fn create(chat_table: &ChatTable) -> Self {
        let message_id = column("message_id", TEXT, [PRIMARY_KEY, NOT_NULL]);
        let chat_id = column(chat_table.chat_id.name(), TEXT, [NOT_NULL]);
        let created_at = column("created_at", INTEGER, [NOT_NULL]);
        let updated_at = column("updated_at", INTEGER, [NOT_NULL]);
        let role = column("role", INTEGER, [NOT_NULL]);
        let text = column("text", TEXT, [NOT_NULL]);
        Self {
            message_id: message_id.clone(),
            chat_id: chat_id.clone(),
            created_at: created_at.clone(),
            updated_at: updated_at.clone(),
            role: role.clone(),
            text: text.clone(),
            columns: vec![
                message_id,
                chat_id.clone(),
                created_at.clone(),
                updated_at,
                role,
                text,
                foreign_key(
                    chat_id,
                    REFERENCES,
                    chat_table,
                    chat_table.chat_id.clone(),
                    [ON_DELETE, CASCADE, DEFERRABLE_INITIALLY_DEFERRED],
                ),
            ],
            indexes: vec![("index_message_created_at".into(), vec![created_at])],
        }
    }
}

impl Table for MessageTable {
    fn name(&self) -> &str {
        "message"
    }

    fn columns(&self) -> &[Arc<Column>] {
        &self.columns
    }

    fn indexes(&self) -> &[(String, Vec<Arc<Column>>)] {
        &self.indexes
    }
}

impl FromSql for ChatID {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        Uuid::parse_str(value.as_str()?)
            .map(Self)
            .map_err(|e| FromSqlError::Other(e.into()))
    }
}

impl ToSql for ChatID {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.0.to_string().into())
    }
}

impl FromSql for MessageID {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        Uuid::parse_str(value.as_str()?)
            .map(Self)
            .map_err(|e| FromSqlError::Other(e.into()))
    }
}

impl ToSql for MessageID {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.0.to_string().into())
    }
}

impl FromSql for MessageRole {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let value = value.as_i64()?;
        match value {
            0 => Ok(Self::System),
            1 => Ok(Self::User),
            2 => Ok(Self::Assistant),
            _ => Err(FromSqlError::Other(
                anyhow::anyhow!("unexpected type: {}", value).into(),
            )),
        }
    }
}

impl ToSql for MessageRole {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self {
            MessageRole::System => 0,
            MessageRole::User => 1,
            MessageRole::Assistant => 2,
        }
        .into())
    }
}

struct RepoDateTime(DateTime<Utc>);

impl FromSql for RepoDateTime {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        Utc.timestamp_millis_opt(value.as_i64()?)
            .single()
            .context("incorrect timestamp millis")
            .map(RepoDateTime)
            .map_err(|e| FromSqlError::Other(e.into()))
    }
}

struct RepoDateTimeRef<'a>(&'a DateTime<Utc>);

impl ToSql for RepoDateTimeRef<'_> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.0.timestamp_millis().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, DurationRound};
    use rusqlite::Row;
    use std::ops::Add;
    use std::str::FromStr;

    #[ignore]
    #[test]
    fn generate_uuid() {
        (0..10).for_each(|_| {
            dbg!(Uuid::new_v4());
        });
    }

    #[ignore]
    #[test]
    fn generate_date_time() {
        (0..10).for_each(|_| {
            dbg!(Utc::now());
        });
    }

    fn create_common_data_set_chat1() -> (Chat, Vec<Message>) {
        (
            Chat {
                chat_id: ChatID(Uuid::from_str("5a753d3b-7d61-492f-9052-3a613500c951").unwrap()),
                title: "My Title foo".to_string(),
                created_at: "2023-05-03T00:31:30.236577Z".parse().unwrap(),
                model_id: "gpt-3.5-turbo".to_string(),
            },
            vec![
                Message {
                    message_id: MessageID(
                        Uuid::from_str("feb75a10-8b2e-4232-ada7-68f92817873d").unwrap(),
                    ),
                    created_at: "2023-05-03T00:31:30.236577Z".parse().unwrap(),
                    updated_at: "2023-05-03T00:31:30.236577Z".parse().unwrap(),
                    role: MessageRole::System,
                    text: "system message".to_string(),
                },
                Message {
                    message_id: MessageID(
                        Uuid::from_str("72aa6cad-576c-4635-bb70-56bb2b945666").unwrap(),
                    ),
                    created_at: "2023-05-03T00:32:31.236577Z".parse().unwrap(),
                    updated_at: "2023-05-03T00:32:31.236577Z".parse().unwrap(),
                    role: MessageRole::User,
                    text: "user message".to_string(),
                },
                Message {
                    message_id: MessageID(
                        Uuid::from_str("51fcfeff-6ccf-453c-90fb-45227d94da32").unwrap(),
                    ),
                    created_at: "2023-05-03T00:33:31.236577Z".parse().unwrap(),
                    updated_at: "2023-05-03T00:33:31.236577Z".parse().unwrap(),
                    role: MessageRole::Assistant,
                    text: "assistant message".to_string(),
                },
            ],
        )
    }

    fn create_common_data_set_chat2() -> (Chat, Vec<Message>) {
        (
            Chat {
                chat_id: ChatID(Uuid::from_str("d27ea7f6-7b83-4971-98e0-2846b699eaad").unwrap()),
                title: "My Title bar".to_string(),
                created_at: "2023-05-03T00:35:17.826251Z".parse().unwrap(),
                model_id: "gpt-3.5-turbo".to_string(),
            },
            vec![],
        )
    }

    fn create_common_data_set_chat3() -> (Chat, Vec<Message>) {
        (
            Chat {
                chat_id: ChatID(Uuid::from_str("4979e325-89d9-441c-86d1-7515a427147a").unwrap()),
                title: "My Title baz".to_string(),
                created_at: "2023-06-03T00:35:17.826251Z".parse().unwrap(),
                model_id: "gpt-3.5-turbo".to_string(),
            },
            vec![
                Message {
                    message_id: MessageID(
                        Uuid::from_str("c0d60ece-44ec-4057-b98a-72abda638365").unwrap(),
                    ),
                    created_at: "2023-05-04T00:33:31.236577Z".parse().unwrap(),
                    updated_at: "2023-05-04T00:33:31.236577Z".parse().unwrap(),
                    role: MessageRole::Assistant,
                    text: "order 3".to_string(),
                },
                Message {
                    message_id: MessageID(
                        Uuid::from_str("0a15391a-8d52-4523-a5e7-4f3f98ff9c5c").unwrap(),
                    ),
                    created_at: "2023-05-03T00:32:31.236577Z".parse().unwrap(),
                    updated_at: "2023-05-03T00:32:31.236577Z".parse().unwrap(),
                    role: MessageRole::User,
                    text: "order 2".to_string(),
                },
                Message {
                    message_id: MessageID(
                        Uuid::from_str("990a5c06-fc2d-4bfc-a3b9-e1c0062fbe71").unwrap(),
                    ),
                    created_at: "2023-05-02T00:31:30.236577Z".parse().unwrap(),
                    updated_at: "2023-05-02T00:31:30.236577Z".parse().unwrap(),
                    role: MessageRole::System,
                    text: "order 1".to_string(),
                },
            ],
        )
    }

    fn create_expected_chat(data: Chat) -> Chat {
        Chat {
            chat_id: data.chat_id,
            title: data.title,
            created_at: data
                .created_at
                .duration_trunc(Duration::milliseconds(1))
                .unwrap(),
            model_id: data.model_id,
        }
    }

    fn create_expected_messages(data: Vec<Message>) -> Vec<Message> {
        let mut ret = data
            .into_iter()
            .map(|data| Message {
                message_id: data.message_id,
                created_at: data
                    .created_at
                    .duration_trunc(Duration::milliseconds(1))
                    .unwrap(),
                updated_at: data
                    .updated_at
                    .duration_trunc(Duration::milliseconds(1))
                    .unwrap(),
                role: data.role,
                text: data.text,
            })
            .collect::<Vec<_>>();
        ret.sort_by_key(|data| data.created_at);
        ret
    }

    fn insert_chats(conn: &mut Connection, chats: &[&Chat]) {
        let chat_table = ChatTable::default();
        let transaction = conn.transaction().unwrap();

        let mut statement = transaction.prepare(
            &format!(
                "insert into {table_name} ({key_chat_id}, {key_title}, {key_created_at}, {key_model_id}) values(:chat_id, :title, :created_at, :model_id)",
                table_name = chat_table.name(),
                key_chat_id = chat_table.chat_id.name(),
                key_title = chat_table.title.name(),
                key_created_at = chat_table.created_at.name(),
                key_model_id = chat_table.model_id.name(),
            )
        ).unwrap();
        for entry in chats {
            statement
                .execute(named_params! {
                    ":chat_id": entry.chat_id,
                    ":title": entry.title,
                    ":created_at": RepoDateTimeRef(&entry.created_at),
                    ":model_id": entry.model_id,
                })
                .unwrap();
        }
        drop(statement);
        transaction.commit().unwrap();
    }

    fn insert_messages(conn: &mut Connection, chat_id: &ChatID, messages: &[Message]) {
        let message_table = MessageTable::create(&ChatTable::default());
        let transaction = conn.transaction().unwrap();

        let mut statement = transaction.prepare(
            &format!("
            insert into {table_name} ({key_message_id},{key_chat_id}, {key_created_at}, {key_updated_at},{key_role}, {key_text}) values(:message_id, :chat_id, :created_at, :updated_at, :role, :text)",
                     table_name = message_table.name(),
                     key_message_id = message_table.message_id.name(),
                     key_chat_id = message_table.chat_id.name(),
                     key_created_at = message_table.created_at.name(),
                     key_updated_at = message_table.updated_at.name(),
                     key_role = message_table.role.name(),
                     key_text = message_table.text.name(),
            )
        ).unwrap();

        for entry in messages {
            statement
                .execute(named_params! {
                    ":message_id": entry.message_id,
                    ":chat_id": chat_id,
                    ":created_at": RepoDateTimeRef(&entry.created_at),
                    ":updated_at": RepoDateTimeRef(&entry.updated_at),
                    ":role": entry.role,
                    ":text": entry.text,
                })
                .unwrap();
        }
        drop(statement);

        transaction.commit().unwrap();
    }

    #[test]
    fn find_chat_all() {
        let (source_chat1, source_chat1_messages) = create_common_data_set_chat1();
        let (source_chat2, source_chat2_messages) = create_common_data_set_chat2();
        let (source_chat3, source_chat3_messages) = create_common_data_set_chat3();

        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("pragma foreign_keys = ON", []).unwrap();

        let user_version = SQLiteUserVersion::from((0, 1, 0));
        conn.execute(
            &format!("pragma user_version = {}", u32::from(user_version.clone())),
            [],
        )
        .unwrap();

        let chat_table = ChatTable::default();
        let message_table = MessageTable::create(&chat_table);

        let transaction = conn.transaction().unwrap();
        transaction.execute(&chat_table.create_sql(), []).unwrap();
        transaction
            .execute(&message_table.create_sql(), [])
            .unwrap();
        transaction.commit().unwrap();

        insert_chats(&mut conn, &[&source_chat1, &source_chat3, &source_chat2]);
        insert_messages(&mut conn, &source_chat1.chat_id, &source_chat1_messages);
        insert_messages(&mut conn, &source_chat3.chat_id, &source_chat3_messages);

        let repo =
            ChatRepositoryImpl::create_with_conn(&FileVersion::from(user_version), conn).unwrap();
        let actual = repo.find_chat_all().unwrap();

        assert_eq!(3, actual.len());
        let (actual_chat1, actual_chat1_messages) = &actual[0];
        assert_eq!(&create_expected_chat(source_chat1), actual_chat1);
        assert_eq!(
            &create_expected_messages(source_chat1_messages),
            actual_chat1_messages,
        );

        let (actual_chat2, actual_chat2_messages) = &actual[1];
        assert_eq!(&create_expected_chat(source_chat2), actual_chat2);
        assert_eq!(&source_chat2_messages, actual_chat2_messages);

        let (actual_chat3, actual_chat3_messages) = &actual[2];
        assert_eq!(&create_expected_chat(source_chat3), actual_chat3);
        assert_eq!(
            &create_expected_messages(source_chat3_messages),
            actual_chat3_messages
        );
    }

    #[test]
    fn find_chat() {
        let (source_chat1, source_chat1_messages) = create_common_data_set_chat1();
        let (source_chat2, source_chat2_messages) = create_common_data_set_chat2();
        let (source_chat3, source_chat3_messages) = create_common_data_set_chat3();

        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("pragma foreign_keys = ON", []).unwrap();

        let user_version = SQLiteUserVersion::from((0, 1, 0));
        conn.execute(
            &format!("pragma user_version = {}", u32::from(user_version.clone())),
            [],
        )
        .unwrap();

        let chat_table = ChatTable::default();
        let message_table = MessageTable::create(&chat_table);
        let transaction = conn.transaction().unwrap();

        transaction.execute(&chat_table.create_sql(), []).unwrap();
        transaction
            .execute(&message_table.create_sql(), [])
            .unwrap();
        transaction.commit().unwrap();

        insert_chats(&mut conn, &[&source_chat1, &source_chat3, &source_chat2]);
        insert_messages(&mut conn, &source_chat1.chat_id, &source_chat1_messages);
        insert_messages(&mut conn, &source_chat3.chat_id, &source_chat3_messages);

        let repo =
            ChatRepositoryImpl::create_with_conn(&FileVersion::from(user_version), conn).unwrap();
        let (actual_chat1, actual_chat1_messages) =
            repo.find_chat(&source_chat1.chat_id).unwrap().unwrap();

        assert_eq!(create_expected_chat(source_chat1), actual_chat1);
        assert_eq!(
            create_expected_messages(source_chat1_messages),
            actual_chat1_messages,
        );

        let (actual_chat2, actual_chat2_messages) =
            repo.find_chat(&source_chat2.chat_id).unwrap().unwrap();
        assert_eq!(create_expected_chat(source_chat2), actual_chat2);
        assert_eq!(source_chat2_messages, actual_chat2_messages);

        let (actual_chat3, actual_chat3_messages) =
            repo.find_chat(&source_chat3.chat_id).unwrap().unwrap();
        assert_eq!(create_expected_chat(source_chat3), actual_chat3);
        assert_eq!(
            create_expected_messages(source_chat3_messages),
            actual_chat3_messages
        );

        let empty = repo
            .find_chat(&ChatID(
                Uuid::from_str("c1dc47f0-64a8-4aba-a17a-00a016219205").unwrap(),
            ))
            .unwrap();
        assert!(empty.is_none());
    }

    fn map_chat(row: &Row) -> rusqlite::Result<Chat> {
        let chat_table = ChatTable::default();
        Ok(Chat {
            chat_id: row.get(chat_table.chat_id.name()).unwrap(),
            title: row.get(chat_table.title.name()).unwrap(),
            created_at: row
                .get::<_, RepoDateTime>(chat_table.created_at.name())
                .unwrap()
                .0,
            model_id: row.get(chat_table.model_id.name()).unwrap(),
        })
    }

    fn map_message(row: &Row) -> rusqlite::Result<Message> {
        let message_table = MessageTable::create(&ChatTable::default());
        Ok(Message {
            message_id: row.get(message_table.message_id.name()).unwrap(),
            created_at: row
                .get::<_, RepoDateTime>(message_table.created_at.name())
                .unwrap()
                .0,
            updated_at: row
                .get::<_, RepoDateTime>(message_table.updated_at.name())
                .unwrap()
                .0,
            role: row.get(message_table.role.name()).unwrap(),
            text: row.get(message_table.text.name()).unwrap(),
        })
    }

    #[test]
    fn save_chat() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("pragma foreign_keys = ON", []).unwrap();

        let user_version = SQLiteUserVersion::from((0, 1, 0));
        conn.execute(
            &format!("pragma user_version = {}", u32::from(user_version.clone())),
            [],
        )
        .unwrap();

        let chat_table = ChatTable::default();
        let message_table = MessageTable::create(&chat_table);
        let transaction = conn.transaction().unwrap();

        transaction.execute(&chat_table.create_sql(), []).unwrap();
        transaction
            .execute(&message_table.create_sql(), [])
            .unwrap();

        transaction.commit().unwrap();

        let (source, _) = create_common_data_set_chat1();

        let repo = ChatRepositoryImpl::create_with_conn(&user_version.into(), conn).unwrap();

        repo.save_chat(&source).unwrap();

        let query_chat = |chat_id: &ChatID| -> Vec<Chat> {
            let conn = repo.get_conn().unwrap();
            let mut statement = conn
                .prepare(&format!(
                    "select * from {table} where {chat_id} = :chat_id",
                    table = chat_table.name(),
                    chat_id = chat_table.chat_id.name(),
                ))
                .unwrap();

            let actual = statement
                .query(named_params! {":chat_id": chat_id})
                .unwrap()
                .mapped(map_chat)
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            drop(statement);
            drop(conn);
            actual
        };

        let actual = query_chat(&source.chat_id);

        let expected = create_expected_chat(source);
        assert_eq!(1, actual.len());
        assert_eq!(expected, actual[0]);

        let source = Chat {
            chat_id: expected.chat_id,
            title: "new title".into(),
            created_at: DateTime::<Utc>::from_str("2023-05-03T11:04:06.328831Z").unwrap(),
            model_id: "new model".into(),
        };

        repo.save_chat(&source).unwrap();

        let actual = query_chat(&source.chat_id);

        assert_eq!(1, actual.len());
        assert_eq!(create_expected_chat(source), actual[0]);
    }

    fn query_messages(conn: &mut Connection, chat_id: &ChatID) -> Vec<Message> {
        let message_table = MessageTable::create(&ChatTable::default());

        let mut statement = conn
            .prepare(&format!(
                "select * from {table} where {chat_id} = :chat_id order by {created_at}",
                table = message_table.name(),
                chat_id = message_table.chat_id.name(),
                created_at = message_table.created_at.name(),
            ))
            .unwrap();

        let actual = statement
            .query(named_params! {":chat_id": chat_id})
            .unwrap()
            .mapped(map_message)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        drop(statement);
        actual
    }

    #[test]
    fn save_messages() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("pragma foreign_keys = ON", []).unwrap();

        let user_version = SQLiteUserVersion::from((0, 1, 0));
        conn.execute(
            &format!("pragma user_version = {}", u32::from(user_version.clone())),
            [],
        )
        .unwrap();

        let chat_table = ChatTable::default();
        let message_table = MessageTable::create(&chat_table);
        let transaction = conn.transaction().unwrap();

        transaction.execute(&chat_table.create_sql(), []).unwrap();
        transaction
            .execute(&message_table.create_sql(), [])
            .unwrap();

        transaction.commit().unwrap();

        let (source_chat, source_messages) = create_common_data_set_chat1();

        let repo = ChatRepositoryImpl::create_with_conn(&user_version.into(), conn).unwrap();

        repo.save_chat(&source_chat).unwrap();
        repo.save_messages(&source_chat.chat_id, &source_messages)
            .unwrap();

        let actual = query_messages(&mut repo.conn.lock().unwrap(), &source_chat.chat_id);

        let expected = create_expected_messages(source_messages);
        assert_eq!(expected.len(), actual.len());
        assert_eq!(expected, actual);

        let source_messages = expected
            .into_iter()
            .enumerate()
            .map(|(i, data)| Message {
                message_id: data.message_id,
                created_at: data.created_at.add(Duration::seconds(1)),
                updated_at: data.updated_at.add(Duration::seconds(1)),
                role: data.role,
                text: format!("new text: {i}"),
            })
            .collect::<Vec<_>>();

        repo.save_messages(&source_chat.chat_id, &source_messages)
            .unwrap();

        let actual = query_messages(&mut repo.conn.lock().unwrap(), &source_chat.chat_id);

        assert_eq!(source_messages.len(), actual.len());
        assert_eq!(source_messages, actual);
    }

    #[test]
    fn delete_messages() {
        let (source_chat1, source_chat1_messages) = create_common_data_set_chat1();
        let (source_chat2, _) = create_common_data_set_chat2();
        let (source_chat3, source_chat3_messages) = create_common_data_set_chat3();

        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute("pragma foreign_keys = ON", []).unwrap();

        let user_version = SQLiteUserVersion::from((0, 1, 0));
        conn.execute(
            &format!("pragma user_version = {}", u32::from(user_version.clone())),
            [],
        )
        .unwrap();

        let chat_table = ChatTable::default();
        let message_table = MessageTable::create(&chat_table);

        let transaction = conn.transaction().unwrap();
        transaction.execute(&chat_table.create_sql(), []).unwrap();
        transaction
            .execute(&message_table.create_sql(), [])
            .unwrap();
        transaction.commit().unwrap();

        insert_chats(&mut conn, &[&source_chat1, &source_chat3, &source_chat2]);
        insert_messages(&mut conn, &source_chat1.chat_id, &source_chat1_messages);
        insert_messages(&mut conn, &source_chat3.chat_id, &source_chat3_messages);

        let repo =
            ChatRepositoryImpl::create_with_conn(&FileVersion::from(user_version), conn).unwrap();

        let affect = repo
            .delete_messages(
                &source_chat1_messages
                    .into_iter()
                    .map(|data| data.message_id)
                    .collect::<Vec<_>>(),
            )
            .unwrap();

        assert_eq!(3, affect);
        assert_eq!(
            0,
            query_messages(&mut repo.conn.lock().unwrap(), &source_chat1.chat_id).len()
        );
        assert_eq!(
            0,
            query_messages(&mut repo.conn.lock().unwrap(), &source_chat2.chat_id).len()
        );
        assert_eq!(
            source_chat3_messages.len(),
            query_messages(&mut repo.conn.lock().unwrap(), &source_chat3.chat_id).len()
        );

        let affect = repo
            .delete_messages(
                &source_chat3_messages
                    .into_iter()
                    .map(|data| data.message_id)
                    .collect::<Vec<_>>(),
            )
            .unwrap();

        assert_eq!(3, affect);
        assert_eq!(
            0,
            query_messages(&mut repo.conn.lock().unwrap(), &source_chat3.chat_id).len()
        );
    }

    #[test]
    #[ignore]
    fn display_indexes() {
        let chat = ChatTable::default();
        let message = MessageTable::create(&chat);

        dbg!(chat.create_index());
        dbg!(message.create_index());
    }
}
