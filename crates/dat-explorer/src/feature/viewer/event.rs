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

use ratatui::crossterm::event::{self, Event};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::mpsc;

/// A single reader owns crossterm input; polling lets it stop without another key press.
pub struct Events {
    pub receiver: mpsc::UnboundedReceiver<std::io::Result<Event>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Events {
    pub fn start() -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = stop.clone();
        let thread = std::thread::Builder::new()
            .name("viewer-input".into())
            .spawn(move || {
                while !signal.load(Ordering::Relaxed) {
                    let result = match event::poll(Duration::from_millis(50)) {
                        Ok(false) => continue,
                        Ok(true) => event::read(),
                        Err(e) => Err(e),
                    };
                    let failed = result.is_err();
                    if sender.send(result).is_err() || failed {
                        break;
                    }
                }
            })?;
        Ok(Self {
            receiver,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
