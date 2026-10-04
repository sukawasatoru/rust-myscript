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

use crate::feature::viewer::popover::{AnchorHit, Popover};
use crate::feature::viewer::text::{LinkTarget, ThreadView};
use crate::model::viewer::{
    BackendKind, DirectoryState, ThreadEntry, ThreadSnapshot, ThreadState, ViewerConfig,
    ViewerState,
};
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use std::path::PathBuf;

pub enum Action {
    None,
    Quit,
    Reload,
    Open(String),
    OpenUrl(String),
    ToggleMouse,
    Save,
    Switch(BackendKind),
}

pub struct App {
    pub directory: PathBuf,
    pub config: ViewerConfig,
    pub state: ViewerState,
    pub entries: Vec<ThreadEntry>,
    pub list: ListState,
    pub thread: Option<ThreadView>,
    pub modal: Option<BackendKind>,
    pub message: String,
    pub loading: Option<(u64, BackendKind)>,
    pub switching: bool,
    pub page_height: usize,
    pub content_area: Rect,
    pub mouse_enabled: bool,
    pub popover: Popover,
    pub redraw: bool,
    request_id: u64,
}

impl App {
    pub fn new(directory: PathBuf, config: ViewerConfig, state: ViewerState) -> Self {
        Self {
            directory,
            config,
            state,
            entries: Vec::new(),
            list: ListState::default(),
            thread: None,
            modal: None,
            message: String::new(),
            loading: None,
            switching: false,
            page_height: 1,
            content_area: Rect::default(),
            mouse_enabled: true,
            popover: Popover::default(),
            redraw: true,
            request_id: 0,
        }
    }

    pub fn begin_request(&mut self) -> u64 {
        self.popover.clear();
        self.redraw = true;
        self.request_id += 1;
        self.loading = Some((self.request_id, self.config.backend));
        self.message.clear();
        self.request_id
    }

    pub fn finish_request(&mut self, id: u64) -> bool {
        if self.loading.is_none_or(|(current, _)| current != id) {
            return false;
        }
        self.loading = None;
        true
    }

    pub fn set_entries(&mut self, entries: Vec<ThreadEntry>) {
        let selected = self
            .list
            .selected()
            .and_then(|i| self.entries.get(i))
            .map(|e| e.file.clone())
            .or_else(|| {
                self.state
                    .directories
                    .iter()
                    .find(|d| d.path == self.directory)
                    .map(|d| d.selected_file.clone())
            });
        let index = selected
            .and_then(|file| entries.iter().position(|e| e.file == file))
            .unwrap_or_else(|| {
                self.list
                    .selected()
                    .unwrap_or(0)
                    .min(entries.len().saturating_sub(1))
            });
        self.list.select((!entries.is_empty()).then_some(index));
        self.entries = entries;
    }

    pub fn set_thread(&mut self, file: String, snapshot: ThreadSnapshot) {
        self.popover.clear();
        self.remember();
        let position = self
            .state
            .threads
            .iter()
            .find(|t| t.directory == self.directory && t.file == file)
            .map(|t| t.position)
            .unwrap_or_default();
        self.thread = Some(ThreadView::new(file, snapshot, position));
    }

    pub fn remember(&mut self) {
        if let Some(entry) = self.list.selected().and_then(|i| self.entries.get(i)) {
            if let Some(state) = self
                .state
                .directories
                .iter_mut()
                .find(|d| d.path == self.directory)
            {
                state.selected_file.clone_from(&entry.file);
            } else {
                self.state.directories.push(DirectoryState {
                    path: self.directory.clone(),
                    selected_file: entry.file.clone(),
                });
            }
        }
        if let Some(thread) = &self.thread {
            if let Some(state) = self
                .state
                .threads
                .iter_mut()
                .find(|t| t.directory == self.directory && t.file == thread.file)
            {
                state.position = thread.position();
            } else {
                self.state.threads.push(ThreadState {
                    directory: self.directory.clone(),
                    file: thread.file.clone(),
                    position: thread.position(),
                });
            }
        }
    }

    pub fn event(&mut self, event: Event) -> Action {
        if !matches!(&event, Event::Mouse(mouse) if mouse.kind == MouseEventKind::Moved) {
            self.redraw = true;
        }
        if matches!(event, Event::Resize(..) | Event::FocusLost) {
            self.popover.clear();
        }
        if let Event::Key(mut key) = event {
            if key.kind == KeyEventKind::Release {
                return Action::None;
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Action::Quit;
            }
            if key.code == KeyCode::F(2) {
                self.popover.clear();
                return if key.kind == KeyEventKind::Press {
                    Action::ToggleMouse
                } else {
                    Action::None
                };
            }
            if key.modifiers == KeyModifiers::CONTROL {
                key.code = match key.code {
                    KeyCode::Char('n') => KeyCode::Down,
                    KeyCode::Char('p') => KeyCode::Up,
                    code => code,
                };
            }
            if let Some(selected) = &mut self.modal {
                match key.code {
                    KeyCode::Esc => {
                        self.modal = None;
                        self.switching = false;
                    }
                    KeyCode::Up | KeyCode::Down if !self.switching => {
                        *selected = match selected {
                            BackendKind::Direct => BackendKind::Mcp,
                            BackendKind::Mcp => BackendKind::Direct,
                        };
                    }
                    KeyCode::Enter if !self.switching => return Action::Switch(*selected),
                    _ => {}
                }
                return Action::None;
            }
            if key.code == KeyCode::Esc {
                let visible = self.popover.close_deepest();
                if visible {
                    return Action::None;
                }
            }
            match key.code {
                KeyCode::F(1) => {
                    self.popover.clear();
                    self.modal = Some(self.config.backend);
                }
                KeyCode::Char(',') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.popover.clear();
                    self.modal = Some(self.config.backend)
                }
                KeyCode::Esc => {
                    if self.thread.is_none() {
                        return Action::Quit;
                    }
                    self.remember();
                    self.thread = None;
                    self.loading = None;
                    self.message.clear();
                    return Action::Save;
                }
                KeyCode::Char('r') => return Action::Reload,
                KeyCode::Enter if self.thread.is_none() => {
                    if let Some(entry) = self.list.selected().and_then(|i| self.entries.get(i)) {
                        return Action::Open(entry.file.clone());
                    }
                }
                KeyCode::Up => self.scroll(-1),
                KeyCode::Down => self.scroll(1),
                KeyCode::PageUp => self.scroll(-(self.page_height as isize)),
                KeyCode::PageDown => self.scroll(self.page_height as isize),
                _ => {}
            }
        } else if let Event::Mouse(mouse) = event
            && self.modal.is_none()
            && self.mouse_enabled
        {
            let point = (mouse.column, mouse.row).into();
            let layer = self.popover.layer_at(point);
            if mouse.kind == MouseEventKind::Moved {
                let hit = self.anchor_at(mouse.column, mouse.row);
                self.redraw |= self
                    .popover
                    .pointer(hit, layer, tokio::time::Instant::now());
                return Action::None;
            }
            if let Some(layer) = layer {
                let hit = self.anchor_at(mouse.column, mouse.row);
                self.popover
                    .pointer(hit, Some(layer), tokio::time::Instant::now());
                let preview = &self.popover.open[layer];
                match mouse.kind {
                    MouseEventKind::ScrollUp => self.popover.scroll(layer, -3),
                    MouseEventKind::ScrollDown => self.popover.scroll(layer, 3),
                    MouseEventKind::Down(MouseButton::Left) => {
                        if preview.inner.contains(point)
                            && let Some(line) = preview
                                .lines
                                .get(preview.top + usize::from(mouse.row - preview.inner.y))
                            && let Some(link) = line.links.iter().find(|link| {
                                link.columns
                                    .contains(&usize::from(mouse.column - preview.inner.x))
                            })
                            && let LinkTarget::Url(url) = &link.target
                        {
                            return Action::OpenUrl(url.clone());
                        }
                    }
                    _ => {}
                }
                // The entire frame captures input; never activate a link behind it.
                return Action::None;
            }
            match mouse.kind {
                MouseEventKind::ScrollUp => self.scroll(-3),
                MouseEventKind::ScrollDown => self.scroll(3),
                MouseEventKind::Down(MouseButton::Left) => {
                    if self.content_area.contains((mouse.column, mouse.row).into())
                        && let Some(thread) = &self.thread
                        && let Some(line) = thread
                            .lines
                            .get(thread.top + usize::from(mouse.row - self.content_area.y))
                        && let Some(link) = line.links.iter().find(|link| {
                            link.columns
                                .contains(&usize::from(mouse.column - self.content_area.x))
                        })
                        && let LinkTarget::Url(url) = &link.target
                    {
                        return Action::OpenUrl(url.clone());
                    }
                }
                _ => {}
            }
        }
        Action::None
    }

    fn scroll(&mut self, amount: isize) {
        self.popover.clear();
        if let Some(thread) = &mut self.thread {
            thread.scroll(amount);
        } else if !self.entries.is_empty() {
            let next = self
                .list
                .selected()
                .unwrap_or(0)
                .saturating_add_signed(amount)
                .min(self.entries.len() - 1);
            self.list.select(Some(next));
        }
    }

    fn anchor_at(&self, column: u16, row: u16) -> Option<AnchorHit> {
        let point = (column, row).into();
        let (lines, top, area) = if let Some(layer) = self.popover.layer_at(point) {
            let preview = &self.popover.open[layer];
            (&preview.lines, preview.top, preview.inner)
        } else {
            let thread = self.thread.as_ref()?;
            (&thread.lines, thread.top, self.content_area)
        };
        if !area.contains(point) {
            return None;
        }
        let line = lines.get(top + usize::from(row - area.y))?;
        let link = line
            .links
            .iter()
            .find(|link| link.columns.contains(&usize::from(column - area.x)))?;
        let LinkTarget::Post(target) = link.target else {
            return None;
        };
        Some(AnchorHit {
            source_post: line.position.post_number,
            source_offset: link.source_offset,
            target,
            area: Rect::new(
                area.x + link.columns.start as u16,
                row,
                (link.columns.end - link.columns.start) as u16,
                1,
            ),
        })
    }

    pub fn advance_popover(&mut self, now: tokio::time::Instant) {
        if let Some(thread) = &self.thread {
            self.redraw |= self.popover.advance(now, thread, self.content_area);
        } else {
            self.redraw |= self.popover.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::viewer::{ReadingPosition, ViewerPost};
    use ratatui::crossterm::event::{KeyEvent, MouseEvent};

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn app() -> App {
        let mut app = App::new(
            "/dat".into(),
            ViewerConfig::default(),
            ViewerState::default(),
        );
        app.set_entries(
            ["new.dat", "old.dat"]
                .into_iter()
                .map(|file| ThreadEntry {
                    file: file.into(),
                    title: file.into(),
                    post_count: 2,
                    created_at: None,
                })
                .collect(),
        );
        app
    }

    fn snapshot() -> ThreadSnapshot {
        ThreadSnapshot {
            title: "title".into(),
            posts: vec![ViewerPost {
                number: 3,
                name: "name".into(),
                datetime: "date".into(),
                id: "id".into(),
                body: "本文".repeat(100),
            }],
        }
    }

    #[test]
    fn control_n_and_p_move_in_both_screens_and_modal() {
        let mut app = app();
        let next = Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
        let previous = Event::Key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        app.event(next.clone());
        assert_eq!(app.list.selected(), Some(1));
        app.event(previous.clone());
        assert_eq!(app.list.selected(), Some(0));
        app.set_thread("new.dat".into(), snapshot());
        app.thread.as_mut().unwrap().reflow(20);
        app.event(next.clone());
        assert_eq!(app.thread.as_ref().unwrap().top, 1);
        app.event(previous.clone());
        assert_eq!(app.thread.as_ref().unwrap().top, 0);
        app.event(key(KeyCode::F(1)));
        app.event(next);
        assert_eq!(app.modal, Some(BackendKind::Mcp));
        app.event(previous);
        assert_eq!(app.modal, Some(BackendKind::Direct));
        assert_eq!(app.thread.as_ref().unwrap().top, 0);
    }

    #[test]
    fn clicks_use_rendered_viewport_and_scroll_offset_and_respect_modal() {
        use crate::feature::viewer::ui;
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = app();
        let mut snapshot = snapshot();
        snapshot.posts[0].body =
            "日本語 https://example.com/very/long/path/to/resource?q=one&b=two\nリンク以外".into();
        app.set_thread("new.dat".into(), snapshot);
        for width in [30, 45] {
            let mut terminal = Terminal::new(TestBackend::new(width, 10)).unwrap();
            terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
            let thread = app.thread.as_mut().unwrap();
            let first = thread
                .lines
                .iter()
                .position(|line| !line.links.is_empty())
                .unwrap();
            thread.top = first + 1; // Only the continuation of the wrapped URL is visible.
            terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
            let thread = app.thread.as_ref().unwrap();
            let link = &thread.lines[thread.top].links[0];
            let LinkTarget::Url(url) = link.target.clone() else {
                panic!("expected URL");
            };
            let column = app.content_area.x + link.columns.start as u16;
            let row = app.content_area.y;
            let mouse = |kind, column, row| {
                Event::Mouse(MouseEvent {
                    kind,
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                })
            };
            let position = thread.position();
            assert!(matches!(app.event(key(KeyCode::F(2))), Action::ToggleMouse));
            app.mouse_enabled = false;
            assert!(matches!(
                app.event(mouse(MouseEventKind::Down(MouseButton::Left), column, row)),
                Action::None
            ));
            app.event(mouse(MouseEventKind::ScrollDown, column, row));
            assert_eq!(app.thread.as_ref().unwrap().position(), position);
            app.event(key(KeyCode::Down));
            app.event(key(KeyCode::Up));
            assert_eq!(app.thread.as_ref().unwrap().position(), position);
            assert!(matches!(app.event(key(KeyCode::F(2))), Action::ToggleMouse));
            app.mouse_enabled = true;
            assert!(
                matches!(app.event(mouse(MouseEventKind::Down(MouseButton::Left), column, row)), Action::OpenUrl(target) if target == url)
            );
            assert_eq!(app.thread.as_ref().unwrap().position(), position);
            assert!(matches!(
                app.event(mouse(MouseEventKind::Down(MouseButton::Right), column, row)),
                Action::None
            ));
            assert!(matches!(
                app.event(mouse(MouseEventKind::Up(MouseButton::Left), column, row)),
                Action::None
            ));
            assert!(matches!(
                app.event(mouse(MouseEventKind::Down(MouseButton::Left), column, 0)),
                Action::None
            ));
            assert!(matches!(
                app.event(mouse(
                    MouseEventKind::Down(MouseButton::Left),
                    column,
                    app.content_area.bottom()
                )),
                Action::None
            ));
            app.event(key(KeyCode::F(1)));
            assert!(matches!(
                app.event(mouse(MouseEventKind::Down(MouseButton::Left), column, row)),
                Action::None
            ));
            app.event(key(KeyCode::Esc));
        }
    }

    #[test]
    fn preview_routes_mouse_input_and_renders_above_the_thread() {
        use crate::feature::viewer::ui;
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = app();
        let mut snapshot = snapshot();
        snapshot.posts[0].body = "日本語 >>5 >>99\nhttps://behind.example\n".repeat(20);
        snapshot.posts.push(ViewerPost {
            number: 5,
            name: "target".into(),
            datetime: "date".into(),
            id: "id".into(),
            body: format!("https://preview.example\n>>3\n{}", "長い本文\n".repeat(40)),
        });
        app.set_thread("new.dat".into(), snapshot);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let mouse = |kind, column, row| {
            Event::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            })
        };
        let hit = app.anchor_at(7, 2).unwrap();
        assert_eq!(hit.target, 5);
        assert!(app.anchor_at(11, 2).is_none());
        app.redraw = false;
        app.event(mouse(MouseEventKind::Moved, 7, 2));
        assert!(!app.redraw);
        assert!(app.popover.open.is_empty());
        app.advance_popover(app.popover.deadline().unwrap());
        assert!(app.redraw);
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        let preview = app.popover.open.last().unwrap();
        let area = preview.area;
        let inner = preview.inner;
        assert_eq!(terminal.backend().buffer()[(area.x, area.y)].symbol(), "┌");
        // A URL is behind the top border, but clicking it must not reach the thread.
        assert!(matches!(
            app.event(mouse(
                MouseEventKind::Down(MouseButton::Left),
                area.x,
                area.y
            )),
            Action::None
        ));
        assert!(
            matches!(app.event(mouse(MouseEventKind::Down(MouseButton::Left), inner.x, inner.y + 1)), Action::OpenUrl(url) if url == "https://preview.example")
        );
        app.event(mouse(MouseEventKind::Moved, inner.x, inner.y + 2));
        assert!(app.popover.deadline().is_some()); // >>3 opens a child after its delay.
        let position = app.thread.as_ref().unwrap().position();
        app.event(mouse(MouseEventKind::ScrollDown, inner.x, inner.y));
        assert_eq!(app.popover.open.last().unwrap().top, 3);
        assert_eq!(app.thread.as_ref().unwrap().position(), position);
        app.event(mouse(MouseEventKind::ScrollDown, 0, 1));
        assert!(app.popover.open.is_empty());
        assert_ne!(app.thread.as_ref().unwrap().position(), position);
    }

    #[test]
    fn nested_previews_overlap_route_to_frontmost_and_preserve_parent_scroll() {
        use crate::feature::viewer::ui;
        use ratatui::{Terminal, backend::TestBackend};

        fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
            Event::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            })
        }

        fn anchor_position(app: &App, layer: Option<usize>, target: usize) -> (u16, u16) {
            let (lines, top, area) = if let Some(layer) = layer {
                let preview = &app.popover.open[layer];
                (&preview.lines, preview.top, preview.inner)
            } else {
                let thread = app.thread.as_ref().unwrap();
                (&thread.lines, thread.top, app.content_area)
            };
            for (row, line) in lines
                .iter()
                .skip(top)
                .take(usize::from(area.height))
                .enumerate()
            {
                if let Some(link) = line
                    .links
                    .iter()
                    .find(|link| link.target == LinkTarget::Post(target))
                {
                    return (area.x + link.columns.start as u16, area.y + row as u16);
                }
            }
            panic!("visible anchor not found");
        }

        let mut app = app();
        let mut snapshot = snapshot();
        snapshot.posts[0].body = ">>5".into();
        for (number, body) in [
            (5, ">>7 >>9\nhttps://parent.example"),
            (7, ">>9\nhttps://child.example"),
            (9, ">>5\nhttps://grandchild.example"),
        ] {
            snapshot.posts.push(ViewerPost {
                number,
                name: "target".into(),
                datetime: "date".into(),
                id: "id".into(),
                body: format!("{body}\n{}", "長い本文\n".repeat(40)),
            });
        }
        app.set_thread("new.dat".into(), snapshot);
        let mut terminal = Terminal::new(TestBackend::new(100, 32)).unwrap();
        terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        for (layer, target) in [(None, 5), (Some(0), 7), (Some(1), 9)] {
            let (x, y) = anchor_position(&app, layer, target);
            app.event(mouse(MouseEventKind::Moved, x, y));
            app.advance_popover(app.popover.deadline().unwrap());
            terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
        }
        assert_eq!(app.popover.open.len(), 3);
        assert!(
            app.popover.open[0]
                .area
                .intersects(app.popover.open[1].area)
        );
        assert!(
            app.popover.open[1]
                .area
                .intersects(app.popover.open[2].area)
        );
        let grandchild = &app.popover.open[2];
        let area = grandchild.area;
        let inner = grandchild.inner;
        assert_eq!(terminal.backend().buffer()[(area.x, area.y)].symbol(), "┌");
        assert!(
            matches!(app.event(mouse(MouseEventKind::Down(MouseButton::Left), inner.x, inner.y + 2)), Action::OpenUrl(url) if url == "https://grandchild.example")
        );
        // An ancestor reference is recognized but creates no duplicate preview.
        let (x, y) = anchor_position(&app, Some(2), 5);
        app.event(mouse(MouseEventKind::Moved, x, y));
        assert_eq!(app.popover.deadline(), None);
        assert_eq!(app.popover.open.len(), 3);
        app.event(mouse(MouseEventKind::ScrollDown, inner.x, inner.y));
        assert_eq!(app.popover.open[2].top, 3);
        assert_eq!(app.popover.open[1].top, 0);
        assert_eq!(app.popover.open[0].top, 0);
        assert_eq!(app.thread.as_ref().unwrap().top, 0);
        app.event(key(KeyCode::Esc));
        assert_eq!(app.popover.open.len(), 2);
        app.event(key(KeyCode::Esc));
        assert_eq!(app.popover.open.len(), 1);
        assert!(app.thread.is_some());
        // Reopen a child, then scroll the exposed parent: descendants close, only parent scrolls.
        let (x, y) = anchor_position(&app, Some(0), 7);
        app.event(mouse(MouseEventKind::Moved, x, y));
        app.advance_popover(app.popover.deadline().unwrap());
        assert_eq!(app.popover.open.len(), 2);
        app.event(mouse(MouseEventKind::ScrollDown, x, y));
        assert_eq!(app.popover.open.len(), 1);
        assert_eq!(app.popover.open[0].top, 3);
        assert_eq!(app.popover.deadline(), None);
    }

    #[test]
    fn navigation_and_lifecycle_changes_cancel_pending_and_visible_previews() {
        use crate::feature::viewer::ui;
        use ratatui::{Terminal, backend::TestBackend};
        for visible in [false, true] {
            for event in [
                key(KeyCode::Esc),
                key(KeyCode::F(1)),
                key(KeyCode::F(2)),
                key(KeyCode::Down),
                Event::Resize(40, 10),
            ] {
                let mut app = app();
                let mut snapshot = snapshot();
                snapshot.posts[0].body = ">>3\n本文".into();
                app.set_thread("new.dat".into(), snapshot);
                let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
                terminal.draw(|frame| ui::draw(frame, &mut app)).unwrap();
                app.event(Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Moved,
                    column: 0,
                    row: 2,
                    modifiers: KeyModifiers::NONE,
                }));
                assert!(app.popover.deadline().is_some());
                if visible {
                    app.advance_popover(app.popover.deadline().unwrap());
                    assert!(!app.popover.open.is_empty());
                }
                let escape = matches!(&event, Event::Key(key) if key.code == KeyCode::Esc);
                app.event(event);
                assert!(app.popover.open.is_empty());
                assert_eq!(app.popover.deadline(), None);
                if visible && escape {
                    assert!(app.thread.is_some());
                    assert!(matches!(app.event(key(KeyCode::Esc)), Action::Save));
                }
            }
        }
        let mut app = app();
        app.set_thread("new.dat".into(), snapshot());
        let hit = AnchorHit {
            source_post: 3,
            source_offset: 0,
            target: 3,
            area: Rect::new(0, 2, 3, 1),
        };
        app.popover
            .pointer(Some(hit.clone()), None, tokio::time::Instant::now());
        app.begin_request();
        assert_eq!(app.popover.deadline(), None);
        app.popover
            .pointer(Some(hit), None, tokio::time::Instant::now());
        app.set_thread("new.dat".into(), snapshot());
        assert_eq!(app.popover.deadline(), None);
    }

    #[test]
    fn modal_owns_escape_and_scroll_and_navigation_preserves_reading_position() {
        let mut app = app();
        app.event(key(KeyCode::Down));
        assert!(matches!(app.event(key(KeyCode::Enter)), Action::Open(file) if file == "old.dat"));
        app.set_thread("old.dat".into(), snapshot());
        app.thread.as_mut().unwrap().reflow(20);
        let mouse = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 1,
            row: 1,
            modifiers: KeyModifiers::NONE,
        });
        app.event(mouse.clone());
        let position = app.thread.as_ref().unwrap().position();
        assert_ne!(position, ReadingPosition::default());
        assert_eq!(app.thread.as_ref().unwrap().top, 3);
        app.event(Event::Key(KeyEvent::new(
            KeyCode::Char(','),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(app.modal, Some(BackendKind::Direct));
        app.event(mouse);
        app.event(key(KeyCode::Down));
        assert_eq!(app.thread.as_ref().unwrap().position(), position);
        assert!(matches!(
            app.event(key(KeyCode::Enter)),
            Action::Switch(BackendKind::Mcp)
        ));
        app.switching = true;
        assert!(matches!(app.event(key(KeyCode::Esc)), Action::None));
        assert!(!app.switching);
        assert!(app.thread.is_some());
        let id = app.begin_request();
        assert!(matches!(app.event(key(KeyCode::Esc)), Action::Save));
        assert!(!app.finish_request(id));
        assert_eq!(app.list.selected(), Some(1));
        app.set_thread("old.dat".into(), snapshot());
        app.thread.as_mut().unwrap().reflow(20);
        assert_eq!(app.thread.as_ref().unwrap().position(), position);
        app.event(key(KeyCode::Esc));
        assert!(matches!(app.event(key(KeyCode::Esc)), Action::Quit));
    }

    #[test]
    fn latest_request_wins_and_refresh_keeps_selection_and_current_reading_position() {
        let mut app = app();
        let first = app.begin_request();
        let second = app.begin_request();
        assert!(!app.finish_request(first));
        assert!(app.finish_request(second));
        app.event(key(KeyCode::Down));
        let mut entries = app.entries.clone();
        entries.reverse();
        app.set_entries(entries);
        assert_eq!(app.list.selected(), Some(0));
        assert_eq!(app.entries[0].file, "old.dat");
        app.set_thread("old.dat".into(), snapshot());
        app.thread.as_mut().unwrap().reflow(20);
        app.begin_request();
        app.event(key(KeyCode::Down));
        let position = app.thread.as_ref().unwrap().position();
        app.set_thread("old.dat".into(), snapshot());
        app.thread.as_mut().unwrap().reflow(20);
        assert_eq!(app.thread.as_ref().unwrap().position(), position);
    }
}
