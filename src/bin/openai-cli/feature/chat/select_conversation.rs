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

use crate::model::{Chat, ChatID, Message, MessageRole};
use chrono::{DateTime, Local, Offset, TimeZone};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap};
use rust_myscript::prelude::*;
use std::fmt::Display;
use std::io;
use std::io::prelude::*;
use tracing::instrument;

pub enum SelectedType {
    New,
    History(ChatID),
    Cancelled,
}

#[instrument(skip_all)]
pub fn select_conversation(conversations: &[(Chat, Vec<Message>)]) -> Fallible<SelectedType> {
    let time_zone = Local::now();
    let time_zone = time_zone.offset();
    let mut items = vec![ConversationType::New];
    items.extend(conversations.iter().map(|(chat, messages)| {
        ConversationType::Continue(ConversationHistory {
            id: chat.chat_id.clone(),
            title: chat.title.clone(),
            created_at: chat.created_at.with_timezone(time_zone),
            updated_at: messages
                .last()
                .map(|data| data.updated_at.with_timezone(time_zone))
                .unwrap_or(chat.created_at.with_timezone(time_zone)),
            messages,
        })
    }));

    let terminal = Terminal::new(CrosstermBackend::new(io::stderr()))?;
    enable_raw_mode()?;
    let mut terminal = SelectConversationTerminal(terminal);
    execute!(io::stderr(), EnterAlternateScreen, EnableMouseCapture)?;

    let mut state = ConversationViewState::new(items);
    loop {
        terminal.0.draw(|f| conversation_view(f, &mut state))?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                    return Ok(SelectedType::Cancelled);
                }
                KeyCode::Char('q') | KeyCode::Esc => return Ok(SelectedType::Cancelled),
                KeyCode::Enter => {
                    return match state
                        .items
                        .swap_remove(state.chat_state.selected().context("state.selected")?)
                    {
                        ConversationType::New => Ok(SelectedType::New),
                        ConversationType::Continue(data) => Ok(SelectedType::History(data.id)),
                    };
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let chat_index = state.chat_state.selected().unwrap_or_else(|| {
                        warn!("chat doesn't selected");
                        0
                    });
                    if let Some(message_index) = state.message_state.selected() {
                        // update messages index.
                        match &state.items[chat_index] {
                            ConversationType::New => {
                                warn!(%chat_index, %message_index, "unexpected");
                            }
                            ConversationType::Continue(data) => {
                                let i = match message_index {
                                    i if data.messages.len() - 1 == i => 0,
                                    i => i + 1,
                                };
                                state.message_state.select(Some(i));
                            }
                        }
                    } else {
                        // update chat/messages index.
                        let i = match chat_index {
                            i if state.items.len() - 1 == i => 0,
                            i => i + 1,
                        };
                        state.chat_state.select(Some(i));
                        state.message_state.select(None);
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let chat_index = state.chat_state.selected().unwrap_or_else(|| {
                        warn!("chat doesn't selected");
                        0
                    });
                    if let Some(message_index) = state.message_state.selected() {
                        // update messages index.
                        match &state.items[chat_index] {
                            ConversationType::New => {
                                warn!(%chat_index, %message_index, "unexpected");
                            }
                            ConversationType::Continue(data) => {
                                let i = match message_index {
                                    0 => data.messages.len() - 1,
                                    i => i - 1,
                                };
                                state.message_state.select(Some(i));
                            }
                        }
                    } else {
                        // update chat/messages index.
                        let i = match chat_index {
                            0 => state.items.len() - 1,
                            i => i - 1,
                        };
                        state.chat_state.select(Some(i));
                        state.message_state.select(None);
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if state.message_state.selected().is_some() {
                        state.message_state.select(None);
                    }
                }
                KeyCode::Right | KeyCode::Char('l') => match state.message_state.selected() {
                    Some(_) => (),
                    None => match state.chat_state.selected() {
                        Some(chat_index) => match &state.items[chat_index] {
                            ConversationType::New => (),
                            ConversationType::Continue(_) => {
                                state.message_state.select(Some(0));
                            }
                        },
                        None => {
                            warn!("chat should selected always");
                        }
                    },
                },
                _ => info!(?key),
            }
        }
    }
}

struct SelectConversationTerminal<B: Backend + Write>(Terminal<B>);

impl<B: Backend + Write> Drop for SelectConversationTerminal<B> {
    fn drop(&mut self) {
        let ret = disable_raw_mode();
        if let Err(e) = ret {
            warn!(?e, "disable_raw_mode");
        }

        let ret = execute!(
            self.0.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture,
        );
        if let Err(e) = ret {
            warn!(?e, "execute!");
        }

        let ret = self.0.show_cursor();
        if let Err(e) = ret {
            warn!(?e, "show_cursor");
        }
    }
}

fn conversation_view<Tz, OFFSET>(f: &mut Frame, state: &mut ConversationViewState<Tz, OFFSET>)
where
    Tz: TimeZone<Offset = OFFSET>,
    OFFSET: Offset + Display,
{
    let chat_widget = List::new(
        state
            .items
            .iter()
            .map(|item| match item {
                ConversationType::New => ListItem::new(Text::from("Start new conversation\n\n\n")),
                ConversationType::Continue(item) => ListItem::new(vec![
                    Line::from(item.title.as_str()),
                    Line::from(vec![
                        Span::from("  created: "),
                        Span::from(item.created_at.to_rfc3339()),
                    ]),
                    Line::from(vec![
                        Span::from("  updated: "),
                        Span::from(item.updated_at.to_rfc3339()),
                    ]),
                ]),
            })
            .collect::<Vec<_>>(),
    )
    .block(
        Block::default()
            .title(" Select an item or 'q' as quit ")
            .borders(Borders::ALL - Borders::RIGHT)
            .border_type(BorderType::Rounded),
    )
    .highlight_style(
        Style::default()
            .bg(Color::White)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    let selected_chat = &state.items[state.chat_state.selected().expect("chat_state.selected")];

    let message_list_widget = List::new(match selected_chat {
        ConversationType::New => vec![],
        ConversationType::Continue(data) => data
            .messages
            .iter()
            .map(|data| {
                ListItem::new(Line::from(match data.role {
                    MessageRole::System => {
                        vec![
                            Span::styled(
                                "system:",
                                Style::default().add_modifier(Modifier::UNDERLINED),
                            ),
                            Span::from("    "),
                            Span::from(data.text.as_str()),
                        ]
                    }
                    MessageRole::User => vec![
                        Span::styled("user:", Style::default().add_modifier(Modifier::UNDERLINED)),
                        Span::from("      "),
                        Span::from(data.text.as_str()),
                    ],
                    MessageRole::Assistant => vec![
                        Span::styled(
                            "assistant:",
                            Style::default().add_modifier(Modifier::UNDERLINED),
                        ),
                        Span::from(" "),
                        Span::from(data.text.as_str()),
                    ],
                }))
            })
            .collect::<Vec<_>>(),
    })
    .block(
        Block::default()
            .borders(Borders::ALL - Borders::BOTTOM)
            .border_type(BorderType::Rounded),
    )
    .highlight_style(
        Style::default()
            .bg(Color::White)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
    );

    let message_content_widget = Paragraph::new(match selected_chat {
        ConversationType::New => "",
        ConversationType::Continue(data) => match state.message_state.selected() {
            Some(message_index) => data.messages[message_index].text.as_str(),
            None => "",
        },
    })
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded),
    )
    .wrap(Wrap { trim: true });

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(f.area());
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    f.render_stateful_widget(chat_widget, chunks[0], &mut state.chat_state);
    f.render_stateful_widget(
        message_list_widget,
        right_chunks[0],
        &mut state.message_state,
    );
    f.render_widget(message_content_widget, right_chunks[1]);
}

struct ConversationViewState<'a, Tz, OFFSET>
where
    Tz: TimeZone<Offset = OFFSET>,
    OFFSET: Offset + Display,
{
    items: Vec<ConversationType<'a, Tz, OFFSET>>,
    chat_state: ListState,
    message_state: ListState,
}

impl<'a, Tz, OFFSET> ConversationViewState<'a, Tz, OFFSET>
where
    Tz: TimeZone<Offset = OFFSET>,
    OFFSET: Offset + Display,
{
    fn new(items: Vec<ConversationType<'a, Tz, OFFSET>>) -> Self {
        let mut state = Self {
            items,
            chat_state: Default::default(),
            message_state: Default::default(),
        };

        state.chat_state.select(Some(0));

        state
    }
}

#[derive(Debug)]
struct ConversationHistory<'a, Tz, OFFSET>
where
    Tz: TimeZone<Offset = OFFSET>,
    OFFSET: Offset + Display,
{
    id: ChatID,
    title: String,
    created_at: DateTime<Tz>,
    updated_at: DateTime<Tz>,
    messages: &'a [Message],
}

#[derive(Debug)]
enum ConversationType<'a, Tz, OFFSET>
where
    Tz: TimeZone<Offset = OFFSET>,
    OFFSET: Offset + Display,
{
    New,
    Continue(ConversationHistory<'a, Tz, OFFSET>),
}
