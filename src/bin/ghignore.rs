/*
 * Copyright 2024, 2025 sukawasatoru
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
use directories::ProjectDirs;
use ratatui::crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers, MouseEventKind, poll,
    read,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph};
use rust_myscript::prelude::*;
use std::fs::File;
use std::io::prelude::*;
use std::io::{BufReader, BufWriter, stderr};
use std::ops::{Deref, DerefMut};
use std::panic;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::AtomicBool;
use tracing::instrument;
use tracing_appender::non_blocking::WorkerGuard;

/// Create gitignore based on `https://github.com/github/gitignore`.
#[derive(Debug, Parser)]
struct Opt {
    /// Create a gitignore file with specified template names.
    #[arg(short, long)]
    template_names: Vec<String>,

    /// Output a gitignore to a specified path instead of the stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

fn main() -> Fallible<()> {
    dotenv::dotenv().ok();

    let project_dir =
        ProjectDirs::from("com", "sukawasatoru", "ghignore").expect("no valid home directory");

    let _guard = init_tracing(&project_dir);

    info!("hello");

    let opt = Opt::parse();

    debug!(?opt);

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

    let selected_files = SelectFilesApp::new(&files, opt.template_names.as_slice())
        .run()?
        .iter()
        .map(|index| files[*index].as_str())
        .collect::<Vec<_>>();

    if !selected_files.is_empty() {
        fn write_gitignore(
            mut writer: impl Write,
            repo_dir: &Path,
            selected_files: &[&str],
        ) -> Fallible<()> {
            writeln!(writer, "# Generate: {}", selected_files.join(", "))?;

            for entry in selected_files {
                writeln!(writer, "\n# {}", entry)?;

                let mut reader = BufReader::new(File::open(repo_dir.join(entry))?);

                std::io::copy(&mut reader, &mut writer)?;
            }

            Ok(())
        }

        if let Some(output_path) = opt.output {
            write_gitignore(
                BufWriter::new(File::create(output_path)?),
                &repo_dir,
                selected_files.as_slice(),
            )?;
        } else {
            write_gitignore(std::io::stdout(), &repo_dir, selected_files.as_slice())?;
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

/// Get a log directory based on dirs-dev/directories-rs#81.
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

struct SelectFilesApp<'a> {
    files: Vec<FileEntry<'a>>,
    focus_area: SelectFilesFocusArea,
    files_layout_state: ListState,
    filter_text: String,
}

impl<'a> SelectFilesApp<'a> {
    fn new(files: &'a [String], template_names: &[String]) -> Self {
        let mut files_layout_state = ListState::default();
        files_layout_state.select_first();
        Self {
            files: files
                .iter()
                .map(|name| FileEntry {
                    checked: template_names.iter().any(|data| {
                        let name = name.to_lowercase();
                        let data = data.to_lowercase();
                        name == data || name.trim_end_matches(".gitignore") == data
                    }),
                    name,
                })
                .collect(),
            focus_area: SelectFilesFocusArea::Files,
            files_layout_state,
            filter_text: String::new(),
        }
    }

    #[instrument(skip_all)]
    fn run(&mut self) -> Fallible<Vec<usize>> {
        enable_raw_mode()?;
        execute!(
            stderr(),
            EnterAlternateScreen,
            EnableMouseCapture,
            // currently not affect for Terminal.app on macOS.
            // PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES),
        )?;

        fn restore_ui() {
            static IS_RESTORED: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(false));

            if !IS_RESTORED.swap(true, std::sync::atomic::Ordering::Relaxed) {
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
        loop {
            terminal.draw(|frame| self.select_files_ui(frame))?;
            if self.select_files_handle_events()? {
                break;
            }
        }

        Ok(self
            .files
            .iter()
            .enumerate()
            .filter_map(
                |(index, entry)| {
                    if entry.checked { Some(index) } else { None }
                },
            )
            .collect::<Vec<_>>())
    }

    #[instrument(skip_all)]
    fn select_files_ui(&mut self, frame: &mut Frame) {
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
        .split(frame.area());
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
                Span::styled(" ⌃C ", Style::new().add_modifier(Modifier::REVERSED)),
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
                self.files
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
        let filter_text = self.filter_text.to_lowercase();
        let files_layout_list = List::new(
            self.files
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
        let files_layout_list = if self.focus_area == SelectFilesFocusArea::Files {
            files_layout_list
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
                .highlight_symbol(">")
        } else {
            files_layout_list.highlight_symbol(" ")
        };

        frame.render_stateful_widget(
            files_layout_list,
            files_layout,
            &mut self.files_layout_state,
        );

        // filter_layout.
        frame.render_widget(
            Paragraph::new(vec![
                Line::default(),
                Line::from(vec![Span::from("Filter: "), Span::from(&self.filter_text)]),
            ])
            .block(Block::new().borders(Borders::ALL ^ Borders::TOP)),
            filter_layout,
        );

        // ok_layout.
        frame.render_widget(
            Paragraph::new(Line::from(" [Generate] ").style(
                if self.focus_area == SelectFilesFocusArea::Control {
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
    fn select_files_handle_events(&mut self) -> Fallible<bool> {
        if poll(std::time::Duration::from_secs(60))? {
            match read()? {
                Event::Key(key) => match key.code {
                    KeyCode::Esc => {
                        if self.focus_area == SelectFilesFocusArea::Files {
                            self.filter_text.clear();
                        }
                    }
                    KeyCode::Backspace => {
                        if self.focus_area == SelectFilesFocusArea::Files {
                            self.filter_text.pop();
                        }
                    }
                    KeyCode::Char('h') if key.modifiers == KeyModifiers::CONTROL => {
                        if self.focus_area == SelectFilesFocusArea::Files {
                            self.filter_text.pop();
                        }
                    }
                    KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                        self.clear_all_checks();
                        return Ok(true);
                    }
                    KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                        if self.focus_area == SelectFilesFocusArea::Files {
                            self.files_layout_state.select_next();
                        }
                    }
                    KeyCode::Down => {
                        if self.focus_area == SelectFilesFocusArea::Files {
                            self.files_layout_state.select_next();
                        }
                    }
                    KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                        if self.focus_area == SelectFilesFocusArea::Files {
                            self.files_layout_state.select_previous();
                        }
                    }
                    KeyCode::Up => {
                        if self.focus_area == SelectFilesFocusArea::Files {
                            self.files_layout_state.select_previous();
                        }
                    }
                    KeyCode::Tab => {
                        self.move_control_focus();
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => match self.focus_area {
                        SelectFilesFocusArea::Files => {
                            self.check_item();
                        }
                        SelectFilesFocusArea::Control => {
                            return Ok(true);
                        }
                    },
                    KeyCode::Char(c)
                        if key.modifiers == KeyModifiers::NONE
                            || key.modifiers == KeyModifiers::SHIFT =>
                    {
                        if self.focus_area == SelectFilesFocusArea::Files {
                            self.filter_text.push(c);
                        }
                    }
                    _ => {
                        debug!(?key);
                    }
                },
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        if self.focus_area == SelectFilesFocusArea::Files {
                            self.files_layout_state.select_next();
                        }
                    }
                    MouseEventKind::ScrollUp => {
                        if self.focus_area == SelectFilesFocusArea::Files {
                            self.files_layout_state.select_previous();
                        }
                    }
                    _ => (),
                },
                _ => (),
            }
        }

        Ok(false)
    }

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

    fn clear_all_checks(&mut self) {
        for entry in self.files.iter_mut() {
            entry.checked = false;
        }
    }

    fn move_control_focus(&mut self) {
        match self.focus_area {
            SelectFilesFocusArea::Files => {
                self.focus_area = SelectFilesFocusArea::Control;
            }
            SelectFilesFocusArea::Control => {
                self.focus_area = SelectFilesFocusArea::Files;
            }
        }
    }

    fn check_item(&mut self) {
        if let Some(selected_index) = self.files_layout_state.selected() {
            let filter_text = self.filter_text.to_lowercase();
            let item = self
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

#[derive(Debug)]
struct FileEntry<'a> {
    checked: bool,
    name: &'a str,
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
