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

//! Pre-wrapped lines with response-relative anchors; scrolling never performs IO.

use crate::model::anchors::find_anchors;
use crate::model::urls::find_links;
use crate::model::viewer::{ReadingPosition, ThreadSnapshot, ViewerPost};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::HashMap;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub struct VisualLine {
    pub text: Line<'static>,
    pub position: ReadingPosition,
    pub links: Vec<VisualLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    Url(String),
    Post(usize),
}

struct TextLink {
    range: Range<usize>,
    target: LinkTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualLink {
    pub columns: Range<usize>,
    pub target: LinkTarget,
    pub source_offset: usize,
}

pub struct ThreadView {
    pub file: String,
    pub snapshot: ThreadSnapshot,
    pub lines: Vec<VisualLine>,
    pub top: usize,
    pub post_index: HashMap<usize, usize>,
    width: u16,
    initial_position: ReadingPosition,
}

impl ThreadView {
    pub fn new(file: String, snapshot: ThreadSnapshot, position: ReadingPosition) -> Self {
        let post_index = snapshot
            .posts
            .iter()
            .enumerate()
            .map(|(index, post)| (post.number, index))
            .collect();
        Self {
            file,
            snapshot,
            lines: Vec::new(),
            top: 0,
            post_index,
            width: 0,
            initial_position: position,
        }
    }

    pub fn position(&self) -> ReadingPosition {
        self.lines
            .get(self.top)
            .map_or(self.initial_position, |line| line.position)
    }

    pub fn reflow(&mut self, width: u16) {
        let width = width.max(1);
        if self.width == width {
            return;
        }
        let position = self.position();
        self.lines.clear();
        for post in &self.snapshot.posts {
            self.lines
                .extend(render_post(post, width, &self.post_index));
        }
        self.width = width;
        self.top = self
            .lines
            .iter()
            .rposition(|line| {
                line.position.post_number < position.post_number
                    || (line.position.post_number == position.post_number
                        && line.position.text_offset <= position.text_offset)
            })
            .unwrap_or(0);
        // A removed response or a shorter replacement should reopen at the surviving
        // response's header, rather than at its trailing blank line.
        if let Some(line) = self.lines.get(self.top)
            && (line.position.post_number != position.post_number
                || (self
                    .lines
                    .get(self.top + 1)
                    .is_none_or(|next| next.position.post_number != line.position.post_number)
                    && position.text_offset > line.position.text_offset))
        {
            let number = line.position.post_number;
            self.top = self
                .lines
                .iter()
                .position(|line| line.position.post_number == number)
                .unwrap_or(0);
        }
    }

    pub fn scroll(&mut self, amount: isize) {
        self.top = self
            .top
            .saturating_add_signed(amount)
            .min(self.lines.len().saturating_sub(1));
    }
}

pub fn render_post(
    post: &ViewerPost,
    width: u16,
    post_index: &HashMap<usize, usize>,
) -> Vec<VisualLine> {
    let header = clean(&format!(
        "{} {} {} ID:{}",
        post.number, post.name, post.datetime, post.id
    ));
    let body = clean(&post.body);
    let mut links: Vec<_> = find_links(&body)
        .into_iter()
        .map(|link| TextLink {
            range: link.range,
            target: LinkTarget::Url(link.url),
        })
        .collect();
    links.extend(
        find_anchors(&body)
            .into_iter()
            .filter(|anchor| post_index.contains_key(&anchor.post_number))
            .map(|anchor| TextLink {
                range: anchor.range,
                target: LinkTarget::Post(anchor.post_number),
            }),
    );
    links.sort_by_key(|link| link.range.start);
    for link in &mut links {
        link.range.start += header.len() + 1;
        link.range.end += header.len() + 1;
    }
    let mut lines = Vec::new();
    wrap_post(
        &mut lines,
        &format!("{header}\n{body}\n"),
        post.number,
        header.len(),
        usize::from(width.max(1)),
        &links,
    );
    lines
}

pub fn clean(text: &str) -> String {
    text.replace('\t', "    ")
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect()
}

fn wrap_post(
    lines: &mut Vec<VisualLine>,
    text: &str,
    post_number: usize,
    header_len: usize,
    width: usize,
    links: &[TextLink],
) {
    let mut logical_offset = 0;
    for logical in text.split('\n') {
        let style = if logical_offset <= header_len {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default()
        };
        let mut start = 0;
        let mut columns = 0;
        for (index, grapheme) in logical.grapheme_indices(true) {
            let size = grapheme.width();
            if columns + size > width && index > start {
                lines.push(visual_line(
                    &logical[start..index],
                    style,
                    ReadingPosition {
                        post_number,
                        text_offset: logical_offset + start,
                    },
                    links,
                ));
                start = index;
                columns = 0;
            }
            columns += size;
        }
        lines.push(visual_line(
            &logical[start..],
            style,
            ReadingPosition {
                post_number,
                text_offset: logical_offset + start,
            },
            links,
        ));
        logical_offset += logical.len() + 1;
    }
}

fn visual_line(
    text: &str,
    style: Style,
    position: ReadingPosition,
    links: &[TextLink],
) -> VisualLine {
    let mut spans = Vec::new();
    let mut visible_links = Vec::new();
    let mut cursor = 0;
    let offset = position.text_offset;
    for link in links {
        let start = link.range.start.max(offset);
        let end = link.range.end.min(offset + text.len());
        if start >= end {
            continue;
        }
        let start = start - offset;
        let end = end - offset;
        if cursor < start {
            spans.push(Span::styled(text[cursor..start].to_owned(), style));
        }
        spans.push(Span::styled(
            text[start..end].to_owned(),
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::UNDERLINED),
        ));
        visible_links.push(VisualLink {
            columns: text[..start].width()..text[..end].width(),
            target: link.target.clone(),
            source_offset: link.range.start,
        });
        cursor = end;
    }
    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_owned(), style));
    }
    VisualLine {
        text: Line::from(spans),
        position,
        links: visible_links,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::viewer::ViewerPost;

    fn snapshot() -> ThreadSnapshot {
        ThreadSnapshot {
            title: "タイトル".into(),
            posts: [1, 3]
                .into_iter()
                .map(|number| ViewerPost {
                    number,
                    name: "名無し".into(),
                    datetime: "2026/09/27".into(),
                    id: "abc".into(),
                    body: "日本語の本文👨‍👩‍👧‍👦e\u{301}を折り返す\n次の行\n\n最後".repeat(4),
                })
                .collect(),
        }
    }

    #[test]
    fn links_follow_display_columns_across_wrapping_and_resize() {
        let mut snapshot = snapshot();
        snapshot.posts.truncate(1);
        snapshot.posts[0].name = "https://header.example".into();
        snapshot.posts[0].body = "日本語\t👨‍👩‍👧‍👦 ttps://example.com/very/long/path?q=1&b=2 次 https://5ch.io/test/read.cgi/board/123/ 終".into();
        let mut view = ThreadView::new("file.dat".into(), snapshot, ReadingPosition::default());
        for width in [200, 20, 37] {
            view.reflow(width);
            let mut pieces = Vec::new();
            for line in &view.lines {
                assert!(
                    line.links
                        .iter()
                        .all(|link| link.target != LinkTarget::Url("https://header.example".into()))
                );
                for link in &line.links {
                    assert!(link.columns.start < link.columns.end);
                    assert!(link.columns.end <= usize::from(width));
                    let span = line.text.spans.iter().find(|span| {
                        span.style.add_modifier.contains(Modifier::UNDERLINED)
                            && matches!(&link.target, LinkTarget::Url(url) if url.ends_with("very/long/path?q=1&b=2"))
                    });
                    if let Some(span) = span {
                        pieces.push(span.content.to_string());
                    }
                }
            }
            assert_eq!(pieces.concat(), "ttps://example.com/very/long/path?q=1&b=2");
            if width == 200 {
                let line = view
                    .lines
                    .iter()
                    .find(|line| !line.links.is_empty())
                    .unwrap();
                assert_eq!(line.links[0].columns.start, "日本語    👨‍👩‍👧‍👦 ".width());
                assert_eq!(
                    line.links[0].target,
                    LinkTarget::Url("https://example.com/very/long/path?q=1&b=2".into())
                );
                assert_eq!(
                    line.links[1].target,
                    LinkTarget::Url("https://5ch.io/test/read.cgi/board/123/".into())
                );
            }
        }
    }

    #[test]
    fn only_existing_references_are_links_with_stable_identity_across_wrapping() {
        let mut snapshot = snapshot();
        snapshot.posts[0].body = "日本語\t👨‍👩‍👧‍👦 >>3 >>2 >>99 >>1-3 >>1,3".into();
        let mut view = ThreadView::new(
            "file.dat".into(),
            snapshot.clone(),
            ReadingPosition::default(),
        );
        let mut identity = None;
        for width in [80, 2, 13] {
            view.reflow(width);
            let mut spelling = String::new();
            for line in view
                .lines
                .iter()
                .filter(|line| line.position.post_number == 1)
            {
                for link in &line.links {
                    assert_eq!(link.target, LinkTarget::Post(3));
                    assert!(link.columns.end <= usize::from(width));
                    assert_eq!(
                        *identity.get_or_insert(link.source_offset),
                        link.source_offset
                    );
                }
                for span in &line.text.spans {
                    if span.style.add_modifier.contains(Modifier::UNDERLINED) {
                        spelling.push_str(&span.content);
                    }
                }
            }
            assert_eq!(spelling, ">>3");
        }
        // A reload that adds the missing response activates that reference too.
        let mut added = snapshot.posts[1].clone();
        added.number = 2;
        snapshot.posts.push(added);
        let mut reloaded = ThreadView::new("file.dat".into(), snapshot, ReadingPosition::default());
        reloaded.reflow(80);
        assert!(
            reloaded
                .lines
                .iter()
                .flat_map(|line| &line.links)
                .any(|link| link.target == LinkTarget::Post(2))
        );
    }

    #[test]
    fn japanese_and_graphemes_wrap_without_loss() {
        let mut view = ThreadView::new("file.dat".into(), snapshot(), ReadingPosition::default());
        view.reflow(10);
        assert!(view.lines.iter().all(|line| line.text.width() <= 10));
        let actual: String = view
            .lines
            .iter()
            .flat_map(|line| line.text.spans.iter().map(|span| span.content.as_ref()))
            .collect();
        assert!(actual.contains("👨‍👩‍👧‍👦e\u{301}"));
        assert!(actual.contains("次の行最後"));
        let before = view.lines.len();
        view.reflow(40);
        assert!(view.lines.len() < before);
    }

    #[test]
    fn resume_survives_resize_reload_and_shortened_file() {
        let mut view = ThreadView::new("file.dat".into(), snapshot(), ReadingPosition::default());
        view.reflow(20);
        view.top = view
            .lines
            .iter()
            .position(|line| line.position.post_number == 3 && line.position.text_offset > 100)
            .unwrap();
        let saved = view.position();
        view.reflow(12);
        assert_eq!(view.position().post_number, 3);
        assert!(view.position().text_offset <= saved.text_offset);
        assert!(saved.text_offset - view.position().text_offset < 36);
        let mut reopened = ThreadView::new("file.dat".into(), snapshot(), saved);
        reopened.reflow(20);
        assert_eq!(reopened.position(), saved);

        let mut shorter = snapshot();
        shorter.posts.truncate(1);
        let mut reopened = ThreadView::new("file.dat".into(), shorter, saved);
        reopened.reflow(20);
        assert_eq!(reopened.position().post_number, 1);
        assert_eq!(reopened.top, 0);
        reopened.scroll(isize::MAX);
        assert!(reopened.top < reopened.lines.len());
        reopened.scroll(isize::MIN);
        assert_eq!(reopened.top, 0);
    }
}
