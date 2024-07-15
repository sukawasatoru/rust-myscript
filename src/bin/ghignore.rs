/*
 * Copyright 2024 sukawasatoru
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
use clap::Parser;
use crossterm::event::{KeyModifiers, MouseEventKind};
use directories::ProjectDirs;
use ratatui::crossterm::event::{
    poll, read, DisableMouseCapture, EnableMouseCapture, Event, KeyCode,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph};
use rust_myscript::prelude::*;
use std::fs::File;
use std::io::prelude::*;
use std::io::{stderr, BufReader};
use std::ops::{Deref, DerefMut};
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;
use tracing::instrument;
use tracing_appender::non_blocking::WorkerGuard;

/// Create gitignore based on `https://github.com/github/gitignore`.
#[derive(Parser)]
struct Opt {
    /// [WIP] Create a gitignore file with specified template names.
    #[clap(short, long)]
    template_names: Vec<String>,

    /// [WIP] Output a gitignore to specified path instead of the stdout.
    #[clap(short, long)]
    output: Option<PathBuf>,
}

fn main() -> Fallible<()> {
    dotenv::dotenv().ok();

    let project_dir =
        ProjectDirs::from("com", "sukawasatoru", "ghignore").expect("no valid home directory");

    let _guard = init_tracing(&project_dir);

    info!("hello");

    let _opt = Opt::parse();

    check_git()?;

    let cache_dir = project_dir.cache_dir();
    if !cache_dir.exists() {
        std::fs::create_dir_all(cache_dir).context("failed to create cache directory")?;
    }

    let repo_dir = cache_dir.join("gitignore");
    if repo_dir.exists() {
        debug!("fetch");
        git_fetch(&repo_dir)?;
        debug!("checkout");
        git_checkout(&repo_dir)?;
    } else {
        debug!("clone");
        git_clone(cache_dir)?;
    }

    let files = git_list_gitignore(&repo_dir)?;

    let selected_files = select_files(&files)?
        .iter()
        .map(|index| files[*index].as_str())
        .collect::<Vec<_>>();

    if !selected_files.is_empty() {
        writeln!(
            std::io::stdout(),
            "# Generate: {}",
            selected_files.join(", ")
        )?;

        for entry in selected_files {
            writeln!(std::io::stdout(), "\n# {}", entry)?;

            let mut reader = BufReader::new(File::open(repo_dir.join(entry))?);

            std::io::copy(&mut reader, &mut std::io::stdout())?;
        }
    }

    info!("bye");

    Ok(())
}

fn init_tracing(project_dirs: &ProjectDirs) -> WorkerGuard {
    let log_dir = log_dir(project_dirs);
    if !log_dir.exists() {
        std::fs::create_dir_all(&log_dir).expect("failed to create log directory");
    }

    let (non_blocking, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::never(log_dir, "tracing.log"));

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(non_blocking)
        .init();

    guard
}

/// Get log directory based on dirs-dev/directories-rs#81.
fn log_dir(project_dirs: &ProjectDirs) -> PathBuf {
    if cfg!(target_os = "macos") {
        directories::UserDirs::new()
            .expect("no valid home directory")
            .home_dir()
            .join("Library/Logs")
            .join(project_dirs.project_path())
    } else if cfg!(target_os = "windows") {
        project_dirs.data_dir().to_owned()
    } else {
        project_dirs
            .state_dir()
            .expect("unsupported environment")
            .to_owned()
    }
}

fn check_git() -> Fallible<()> {
    let ret = std::process::Command::new("git")
        .arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?
        .wait()?;
    if ret.code().context("terminated")? != 0 {
        bail!("git not found");
    }

    Ok(())
}

fn git_fetch(repo_path: &Path) -> Fallible<()> {
    let status_code = std::process::Command::new("git")
        .args(["fetch", "origin", "main"])
        .stdin(std::process::Stdio::null())
        .stdout(stderr())
        .stderr(std::process::Stdio::inherit())
        .current_dir(repo_path)
        .spawn()?
        .wait()?;

    match status_code.code() {
        Some(0) => Ok(()),
        Some(_) => {
            bail!("failed to fetch repository: {}", status_code)
        }
        None => bail!("killed git process"),
    }
}

fn git_checkout(repo_path: &Path) -> Fallible<()> {
    let status_code = std::process::Command::new("git")
        .args(["checkout", "origin/main"])
        .stdin(std::process::Stdio::null())
        .stdout(stderr())
        .stderr(std::process::Stdio::inherit())
        .current_dir(repo_path)
        .spawn()?
        .wait()?;

    match status_code.code() {
        Some(0) => Ok(()),
        Some(_) => {
            bail!("failed to checkout repository: {}", status_code)
        }
        None => bail!("killed git process"),
    }
}

fn git_clone(cache_dir: &Path) -> Fallible<()> {
    let status_code = std::process::Command::new("git")
        .args([
            "clone",
            "--filter=blob:none",
            "https://github.com/github/gitignore.git",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(stderr())
        .stderr(std::process::Stdio::inherit())
        .current_dir(cache_dir)
        .spawn()?
        .wait()?;

    match status_code.code() {
        Some(0) => Ok(()),
        Some(_) => {
            bail!("failed to clone repository: {}", status_code)
        }
        None => bail!("killed git process"),
    }
}

fn git_list_gitignore(repo_path: &Path) -> Fallible<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "*.gitignore"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .current_dir(repo_path)
        .spawn()?
        .wait_with_output()?;

    match output.status.code() {
        Some(0) => (),
        Some(_) => {
            bail!("failed to list files: {}", output.status)
        }
        None => bail!("killed git process"),
    }

    let mut file_list = vec![];
    let stdout_string = String::from_utf8(output.stdout)?;
    for line in stdout_string.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        file_list.push(line.to_owned());
    }

    Ok(file_list)
}

struct SelectFilesApp;

impl SelectFilesApp {
    fn filter_file_entry(entry: &FileEntry, filter_text: &str) -> bool {
        if filter_text.is_empty() {
            true
        } else {
            entry
                .name
                .trim_end_matches(".gitignore")
                .to_lowercase()
                .contains(filter_text)
        }
    }

    fn clear_all_checks(view_state: &mut SelectFilesViewState) {
        for entry in view_state.files.iter_mut() {
            entry.checked = false;
        }
    }

    fn move_control_focus(view_state: &mut SelectFilesViewState) {
        match view_state.focus_area {
            SelectFilesFocusArea::Files => {
                view_state.focus_area = SelectFilesFocusArea::Control;
            }
            SelectFilesFocusArea::Control => {
                view_state.focus_area = SelectFilesFocusArea::Files;
            }
        }
    }
    fn check_item(view_state: &mut SelectFilesViewState) {
        if let Some(selected_index) = view_state.files_layout_state.selected() {
            let filter_text = view_state.filter_text.to_lowercase();
            let item = view_state
                .files
                .iter_mut()
                .filter(|entry| SelectFilesApp::filter_file_entry(entry, &filter_text))
                .nth(selected_index)
                .expect("illegal index state");
            item.checked = !item.checked;
            debug!(checked = ?item);
        }
    }
}

fn select_files(all_files: &[String]) -> Fallible<Vec<usize>> {
    enable_raw_mode()?;
    execute!(
        stderr(),
        EnterAlternateScreen,
        EnableMouseCapture,
        // currently not affect for Terminal.app on macOS.
        // PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES),
    )?;

    fn restore_ui() {
        static IS_RESTORED: OnceLock<AtomicBool> = OnceLock::new();
        let is_restored = IS_RESTORED.get_or_init(|| AtomicBool::new(false));

        if !is_restored.swap(true, std::sync::atomic::Ordering::Relaxed) {
            debug!("restore ui");
            if let Err(e) = execute!(
                stderr(),
                LeaveAlternateScreen,
                DisableMouseCapture,
                // PopKeyboardEnhancementFlags,
            ) {
                error!(?e);
            }
            if let Err(e) = disable_raw_mode() {
                error!(?e);
            }
        }
    }

    struct TearDown<B: Backend>(Terminal<B>);
    impl<B: Backend> TearDown<B> {
        fn new(terminal: Terminal<B>) -> Self {
            let original_hook = panic::take_hook();
            panic::set_hook(Box::new(move |info| {
                restore_ui();
                original_hook(info);
            }));

            Self(terminal)
        }
    }
    impl<B: Backend> Drop for TearDown<B> {
        fn drop(&mut self) {
            restore_ui();
        }
    }
    impl<B: Backend> Deref for TearDown<B> {
        type Target = Terminal<B>;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }
    impl<B: Backend> DerefMut for TearDown<B> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.0
        }
    }

    let mut terminal = TearDown::new(Terminal::new(CrosstermBackend::new(stderr()))?);
    let mut view_state = SelectFilesViewState::new(all_files);
    loop {
        terminal.draw(|frame| select_files_ui(frame, &mut view_state))?;
        match select_files_handle_events(&mut view_state)? {
            true => break,
            false => (),
        }
    }

    Ok(view_state
        .files
        .iter()
        .enumerate()
        .filter_map(
            |(index, entry)| {
                if entry.checked {
                    Some(index)
                } else {
                    None
                }
            },
        )
        .collect::<Vec<_>>())
}

#[instrument(skip_all)]
fn select_files_ui(frame: &mut Frame, view_state: &mut SelectFilesViewState) {
    let main_layout = Layout::new(
        Direction::Vertical,
        [
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Length(3),
            Constraint::Length(3),
        ],
    )
    .split(frame.size());
    let help_layout = main_layout[0];
    let selected_file_layout = main_layout[1];
    let files_layout = main_layout[2];
    let filter_layout = main_layout[3];
    let ok_layout = main_layout[4];

    // help_layout.
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ⎋  ", Style::new().add_modifier(Modifier::REVERSED)),
            Span::from(" Clear-text  "),
            Span::styled(" ⌃C/Q ", Style::new().add_modifier(Modifier::REVERSED)),
            Span::from(" Quit  "),
            Span::styled(" ↑/^P ", Style::new().add_modifier(Modifier::REVERSED)),
            Span::from(" Up  "),
            Span::styled(" ↓/^N ", Style::new().add_modifier(Modifier::REVERSED)),
            Span::from(" Down  "),
            Span::styled(" ⇥ ", Style::new().add_modifier(Modifier::REVERSED)),
            Span::from(" Next-control  "),
            Span::styled(" Space/⏎  ", Style::new().add_modifier(Modifier::REVERSED)),
            Span::from(" Check "),
        ]))
        .block(Block::new().borders(Borders::ALL).title("HELP")),
        help_layout,
    );

    // selected_file_layout
    frame.render_widget(
        Paragraph::new(Line::from(
            view_state
                .files
                .iter()
                .filter_map(|entry| {
                    if entry.checked {
                        Some([
                            Span::from(entry.name.trim_end_matches(".gitignore")),
                            Span::from(" "),
                        ])
                    } else {
                        None
                    }
                })
                .flatten()
                .collect::<Vec<_>>(),
        ))
        .block(Block::new().borders(Borders::ALL).title("Selected")),
        selected_file_layout,
    );

    // files_layout.
    let filter_text = view_state.filter_text.to_lowercase();
    let files_layout_list = List::new(
        view_state
            .files
            .iter()
            .filter(|entry| SelectFilesApp::filter_file_entry(entry, &filter_text))
            .map(|entry| {
                ListItem::new(Line::from(vec![
                    if entry.checked {
                        Span::from("[x] ")
                    } else {
                        Span::from("[ ] ")
                    },
                    Span::from(entry.name),
                ]))
            })
            .collect::<Vec<_>>(),
    )
    .block(
        Block::new()
            .borders(Borders::ALL ^ Borders::BOTTOM)
            .title("List"),
    )
    .highlight_spacing(HighlightSpacing::Always);
    let files_layout_list = if view_state.focus_area == SelectFilesFocusArea::Files {
        files_layout_list
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .highlight_symbol(">")
    } else {
        files_layout_list.highlight_symbol(" ")
    };

    frame.render_stateful_widget(
        files_layout_list,
        files_layout,
        &mut view_state.files_layout_state,
    );

    // filter_layout.
    frame.render_widget(
        Paragraph::new(vec![
            Line::default(),
            Line::from(vec![
                Span::from("Filter: "),
                Span::from(&view_state.filter_text),
            ]),
        ])
        .block(Block::new().borders(Borders::ALL ^ Borders::TOP)),
        filter_layout,
    );

    // ok_layout.
    frame.render_widget(
        Paragraph::new(Line::from(" [Generate] ").style(
            if view_state.focus_area == SelectFilesFocusArea::Control {
                Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::default()
            },
        ))
        .alignment(Alignment::Center)
        .block(Block::new().borders(Borders::ALL)),
        ok_layout,
    );
}

#[instrument(skip_all)]
fn select_files_handle_events(view_state: &mut SelectFilesViewState) -> Fallible<bool> {
    if poll(std::time::Duration::from_secs(60))? {
        match read()? {
            Event::Key(key) => match key.code {
                KeyCode::Esc => {
                    if view_state.focus_area == SelectFilesFocusArea::Files {
                        view_state.filter_text.clear();
                    }
                }
                KeyCode::Backspace => {
                    if view_state.focus_area == SelectFilesFocusArea::Files {
                        view_state.filter_text.pop();
                    }
                }
                KeyCode::Char('h') if key.modifiers == KeyModifiers::CONTROL => {
                    if view_state.focus_area == SelectFilesFocusArea::Files {
                        view_state.filter_text.pop();
                    }
                }
                KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                    SelectFilesApp::clear_all_checks(view_state);
                    return Ok(true);
                }
                KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                    if view_state.focus_area == SelectFilesFocusArea::Files {
                        view_state.files_layout_state.select_next();
                    }
                }
                KeyCode::Down => {
                    if view_state.focus_area == SelectFilesFocusArea::Files {
                        view_state.files_layout_state.select_next();
                    }
                }
                KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                    if view_state.focus_area == SelectFilesFocusArea::Files {
                        view_state.files_layout_state.select_previous();
                    }
                }
                KeyCode::Up => {
                    if view_state.focus_area == SelectFilesFocusArea::Files {
                        view_state.files_layout_state.select_previous();
                    }
                }
                KeyCode::Tab => {
                    SelectFilesApp::move_control_focus(view_state);
                }
                KeyCode::Enter | KeyCode::Char(' ') => match view_state.focus_area {
                    SelectFilesFocusArea::Files => {
                        SelectFilesApp::check_item(view_state);
                    }
                    SelectFilesFocusArea::Control => {
                        return Ok(true);
                    }
                },
                KeyCode::Char(c)
                    if key.modifiers == KeyModifiers::NONE
                        || key.modifiers == KeyModifiers::SHIFT =>
                {
                    if view_state.focus_area == SelectFilesFocusArea::Files {
                        view_state.filter_text.push(c);
                    }
                }
                _ => {
                    debug!(?key);
                }
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => {
                    if view_state.focus_area == SelectFilesFocusArea::Files {
                        view_state.files_layout_state.select_next();
                    }
                }
                MouseEventKind::ScrollUp => {
                    if view_state.focus_area == SelectFilesFocusArea::Files {
                        view_state.files_layout_state.select_previous();
                    }
                }
                _ => (),
            },
            _ => (),
        }
    }

    Ok(false)
}

struct SelectFilesViewState<'a> {
    files: Vec<FileEntry<'a>>,
    focus_area: SelectFilesFocusArea,
    files_layout_state: ListState,
    filter_text: String,
}

#[derive(Debug)]
struct FileEntry<'a> {
    checked: bool,
    name: &'a str,
}

impl<'a> SelectFilesViewState<'a> {
    fn new(files: &'a [String]) -> SelectFilesViewState<'a> {
        let mut files_layout_state = ListState::default();
        files_layout_state.select_first();
        Self {
            files: files
                .iter()
                .map(|name| FileEntry {
                    checked: false,
                    name,
                })
                .collect(),
            focus_area: SelectFilesFocusArea::Files,
            files_layout_state,
            filter_text: String::new(),
        }
    }
}

#[derive(Eq, PartialEq)]
enum SelectFilesFocusArea {
    Files,
    Control,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Opt::command().debug_assert();
    }
}
