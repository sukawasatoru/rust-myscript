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

//! Two-screen local dat reader. Backend handles are fixed at request start.

mod app;
mod event;
mod popover;
mod text;
mod ui;

use crate::application::backend::Backend;
use crate::application::thread_service::ThreadService;
use crate::data::mcp_client::McpClient;
use crate::data::viewer_store::ViewerStore;
use crate::feature::viewer::app::{Action, App};
use crate::feature::viewer::event::Events;
use crate::model::viewer::{BackendKind, ThreadEntry, ThreadSnapshot};
use ratatui::crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::{execute, terminal};
use rust_myscript::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::JoinSet;

enum Completion {
    List(u64, Fallible<Vec<ThreadEntry>>),
    Thread(u64, String, Fallible<ThreadSnapshot>),
    Connected(Fallible<Arc<McpClient>>),
    BrowserOpened(Fallible<()>),
}

struct Session {
    app: App,
    store: ViewerStore,
    direct: Arc<ThreadService>,
    mcp: Option<Arc<McpClient>>,
    executable: PathBuf,
    jobs: JoinSet<Completion>,
    connecting: bool,
}

pub async fn run(
    directory: PathBuf,
    direct: Arc<ThreadService>,
    store: ViewerStore,
) -> Fallible<()> {
    // Invalid files are reported before starting the terminal, rather than overwritten with defaults.
    let config = store.load_config()?;
    let state = store.load_state()?;
    let mut session = Session {
        app: App::new(directory, config, state),
        store,
        direct,
        mcp: None,
        executable: std::env::current_exe()?,
        jobs: JoinSet::new(),
        connecting: false,
    };
    if session.app.config.backend == BackendKind::Mcp {
        match McpClient::connect(&session.executable, &session.app.directory).await {
            Ok(client) => session.mcp = Some(Arc::new(client)),
            Err(e) => {
                session.app.config.backend = BackendKind::Direct;
                session.app.message = format!("MCP 接続失敗（Direct を使用）: {e:#}");
            }
        }
    }
    let result = session.terminal_loop().await;
    session.app.remember();
    let saved = session.store.save_state(&session.app.state);
    // Closing first releases pending MCP calls without waiting for their request timeout.
    let closed = if let Some(client) = &session.mcp {
        client.close().await
    } else {
        Ok(())
    };
    // Drain connection jobs too, so even an initialization finishing during exit is closed explicitly.
    while let Some(result) = session.jobs.join_next().await {
        if let Ok(Completion::Connected(Ok(client))) = result {
            let _ = client.close().await;
        }
    }
    result?;
    saved?;
    closed
}

impl Session {
    fn backend(&self) -> Backend {
        match self.app.config.backend {
            BackendKind::Direct => Backend::Direct(self.direct.clone()),
            BackendKind::Mcp => Backend::Mcp(
                self.mcp
                    .as_ref()
                    .expect("selected MCP is initialized")
                    .clone(),
            ),
        }
    }

    fn request(&mut self, file: Option<String>) {
        let backend = self.backend();
        let id = self.app.begin_request();
        self.jobs.spawn(async move {
            match file {
                Some(file) => {
                    let result = backend.load_thread(file.clone()).await;
                    Completion::Thread(id, file, result)
                }
                None => Completion::List(id, backend.list_threads().await),
            }
        });
    }

    fn select_backend(&mut self, kind: BackendKind) {
        self.app.config.backend = kind;
        self.app.modal = None;
        self.app.switching = false;
        self.app.message = match self.store.save_config(&self.app.config) {
            Ok(()) => format!("{}: 次の読み込みから適用", kind.label()),
            Err(e) => format!("設定保存失敗: {e:#}"),
        };
    }

    fn action(&mut self, action: Action) -> bool {
        match action {
            Action::None => {}
            Action::Quit => return true,
            Action::ToggleMouse => {
                let enabled = !self.app.mouse_enabled;
                let result = if enabled {
                    execute!(std::io::stdout(), EnableMouseCapture)
                } else {
                    execute!(std::io::stdout(), DisableMouseCapture)
                };
                match result {
                    Ok(()) => self.app.mouse_enabled = enabled,
                    Err(e) => self.app.message = format!("マウス操作の切り替え失敗: {e}"),
                }
            }
            Action::Reload => self.request(self.app.thread.as_ref().map(|t| t.file.clone())),
            Action::Open(file) => self.request(Some(file)),
            Action::OpenUrl(url) => {
                self.jobs.spawn_blocking(move || {
                    Completion::BrowserOpened(
                        opener::open_browser(url).map_err(anyhow::Error::from),
                    )
                });
            }
            Action::Save => {
                if let Err(e) = self.store.save_state(&self.app.state) {
                    self.app.message = format!("読書位置保存失敗: {e:#}");
                }
            }
            Action::Switch(kind) => {
                if kind == BackendKind::Direct || self.mcp.is_some() {
                    self.select_backend(kind);
                } else {
                    self.app.switching = true;
                    if !self.connecting {
                        self.connecting = true;
                        let executable = self.executable.clone();
                        let directory = self.app.directory.clone();
                        self.jobs.spawn(async move {
                            Completion::Connected(
                                McpClient::connect(&executable, &directory)
                                    .await
                                    .map(Arc::new),
                            )
                        });
                    }
                }
            }
        }
        false
    }

    fn complete(&mut self, completion: Completion) {
        self.app.redraw = true;
        match completion {
            Completion::BrowserOpened(result) => {
                self.app.message = match result {
                    Ok(()) => "ブラウザに URL を渡しました".into(),
                    Err(e) => format!("ブラウザ起動失敗: {e:#}"),
                };
            }
            Completion::List(id, result) if self.app.finish_request(id) => match result {
                Ok(entries) => self.app.set_entries(entries),
                Err(e) => self.app.message = format!("一覧読込失敗: {e:#}"),
            },
            Completion::Thread(id, file, result) if self.app.finish_request(id) => match result {
                Ok(snapshot) => self.app.set_thread(file, snapshot),
                Err(e) => self.app.message = format!("dat 読込失敗: {e:#}"),
            },
            Completion::Connected(result) => {
                self.connecting = false;
                match result {
                    Ok(client) => {
                        self.mcp = Some(client);
                        if self.app.switching {
                            self.select_backend(BackendKind::Mcp);
                        }
                    }
                    Err(e) => self.app.message = format!("MCP 接続失敗: {e:#}"),
                }
                self.app.switching = false;
            }
            _ => {} // A newer request or navigation invalidated this result.
        }
    }

    async fn terminal_loop(&mut self) -> Fallible<()> {
        let mut guard = TerminalGuard { enhanced: false };
        let mut terminal = ratatui::try_init()?;
        execute!(std::io::stdout(), EnableMouseCapture)?;
        guard.enhanced = terminal::supports_keyboard_enhancement().unwrap_or(false);
        if guard.enhanced {
            execute!(
                std::io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }
        let enhanced = guard.enhanced;
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_input(enhanced);
            hook(info);
        }));
        let mut events = Events::start()?;
        let startup_message = std::mem::take(&mut self.app.message);
        self.request(None);
        self.app.message = startup_message;
        loop {
            if self.app.redraw {
                terminal.draw(|frame| ui::draw(frame, &mut self.app))?;
                self.app.redraw = false;
            }
            let deadline = self.app.popover.deadline();
            tokio::select! {
                _ = async {
                    if let Some(deadline) = deadline {
                        tokio::time::sleep_until(deadline).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => self.app.advance_popover(tokio::time::Instant::now()),
                event = events.receiver.recv() => {
                    let event = event.context("terminal input stopped")??;
                    let action = self.app.event(event);
                    if self.action(action) { break; }
                }
                result = self.jobs.join_next(), if !self.jobs.is_empty() => {
                    match result.context("missing background task")? {
                        Ok(completion) => self.complete(completion),
                        Err(e) => { self.app.loading = None; self.app.redraw = true; self.app.message = format!("読込処理失敗: {e}"); }
                    }
                }
                signal = tokio::signal::ctrl_c() => { signal?; break; }
            }
        }
        Ok(())
    }
}

struct TerminalGuard {
    enhanced: bool,
}

fn restore_input(enhanced: bool) {
    if enhanced {
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_input(self.enhanced);
        ratatui::restore();
    }
}
