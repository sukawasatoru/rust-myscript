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

use crate::feature::viewer::app::App;
use crate::feature::viewer::text::clean;
use crate::model::viewer::BackendKind;
use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [header, content, status, help] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    app.page_height = usize::from(content.height).max(1);
    app.content_area = content;
    if let Some(thread) = &mut app.thread {
        thread.reflow(content.width);
        frame.render_widget(
            Paragraph::new(clean(&thread.snapshot.title)).style(Style::default().fg(Color::Yellow)),
            header,
        );
        let lines: Vec<_> = thread
            .lines
            .iter()
            .skip(thread.top)
            .take(usize::from(content.height))
            .map(|line| line.text.clone())
            .collect();
        frame.render_widget(Paragraph::new(lines), content);
    } else {
        frame.render_widget(
            Paragraph::new(format!("スレッド一覧 — {}", app.directory.display()))
                .style(Style::default().fg(Color::Yellow)),
            header,
        );
        let items = app
            .entries
            .iter()
            .map(|entry| {
                let date = entry
                    .created_at
                    .and_then(|time| DateTime::from_timestamp(time, 0))
                    .map(|time| {
                        time.with_timezone(&Local)
                            .format("%Y-%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_else(|| "日時不明        ".into());
                let title = if entry.title.is_empty() {
                    &entry.file
                } else {
                    &entry.title
                };
                ListItem::new(format!(
                    "{date} {:>4}  {}",
                    entry.post_count,
                    clean(title).replace('\n', " ")
                ))
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            frame.render_widget(
                Paragraph::new("dat ファイルがありません。r: 一覧更新"),
                content,
            );
        } else {
            frame.render_stateful_widget(
                List::new(items)
                    .highlight_symbol("› ")
                    .highlight_style(Style::default().bg(Color::DarkGray)),
                content,
                &mut app.list,
            );
        }
    }
    let location = if let Some(thread) = &app.thread {
        format!(
            "レス {} / {} 件",
            thread.position().post_number,
            thread.snapshot.posts.len()
        )
    } else {
        format!("{} 件", app.entries.len())
    };
    let activity = if app.switching {
        "MCP 接続中…".into()
    } else if let Some((_, kind)) = app.loading {
        format!("{} 読込中…", kind.label())
    } else {
        clean(&app.message).replace('\n', " ")
    };
    frame.render_widget(
        Paragraph::new(format!(
            "[{}] [F2 マウス:{}] {location}  {activity}",
            app.config.backend.label(),
            if app.mouse_enabled {
                "ON"
            } else {
                "OFF・文字選択"
            }
        )),
        status,
    );
    frame.render_widget(
        Paragraph::new(if app.thread.is_some() {
            "↑↓/Ctrl+N/P/PgUp/PgDn: 移動  ホバー: レス  クリック: URL  r: 再読込  Esc: 戻る  F1: 設定"
        } else {
            "Enter: 開く  ↑↓/Ctrl+N/P/PgUp/PgDn: 移動  r: 更新  Esc: 終了  F1: 設定"
        }),
        help,
    );

    for preview in &app.popover.open {
        frame.render_widget(Clear, preview.area);
        let scrolling = preview.lines.len() > usize::from(preview.inner.height);
        let title = if scrolling {
            format!(
                "レス {} — {}/{} ホイール: 移動 Esc: 閉じる",
                preview.anchor.target,
                preview.top + 1,
                preview.lines.len()
            )
        } else {
            format!("レス {} — Esc: 閉じる", preview.anchor.target)
        };
        frame.render_widget(Block::bordered().title(title), preview.area);
        let lines: Vec<_> = preview
            .lines
            .iter()
            .skip(preview.top)
            .take(usize::from(preview.inner.height))
            .map(|line| line.text.clone())
            .collect();
        frame.render_widget(Paragraph::new(lines), preview.inner);
    }

    if let Some(selected) = app.modal {
        let area = frame.area();
        let width = area.width.min(54);
        let height = area.height.min(7);
        let popup = ratatui::layout::Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, popup);
        let block = Block::bordered().title("取得経路");
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        let [choices, help] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(inner);
        let mut state =
            ListState::default().with_selected(Some(usize::from(selected == BackendKind::Mcp)));
        frame.render_stateful_widget(
            List::new([
                "Direct — 共通処理を直接呼び出す",
                "MCP — ローカル子プロセス経由",
            ])
            .highlight_symbol("› ")
            .highlight_style(Style::default().bg(Color::DarkGray)),
            choices,
            &mut state,
        );
        frame.render_widget(
            Paragraph::new(vec![
                Line::raw(if app.switching {
                    "MCP 接続中… Esc: キャンセル"
                } else {
                    "↑↓/Ctrl+N/P: 選択  Enter: 確定  Esc: 閉じる"
                }),
                Line::raw("次の読み込み操作から適用"),
            ]),
            help,
        );
    }
}
