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

//! One non-modal response preview. All deadlines are driven by the terminal loop.

use crate::feature::viewer::text::{ThreadView, VisualLine, render_post};
use ratatui::layout::{Position, Rect};
use std::time::Duration;
use tokio::time::Instant;

const OPEN_DELAY: Duration = Duration::from_millis(150);
const CLOSE_DELAY: Duration = Duration::from_millis(200);
const MAX_WIDTH: u16 = 72;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorHit {
    pub source_post: usize,
    pub source_offset: usize,
    pub target: usize,
    pub area: Rect,
}

impl AnchorHit {
    fn same_anchor(&self, other: &Self) -> bool {
        self.source_post == other.source_post && self.source_offset == other.source_offset
    }
}

pub struct Preview {
    pub anchor: AnchorHit,
    pub area: Rect,
    pub inner: Rect,
    pub lines: Vec<VisualLine>,
    pub top: usize,
}

impl Preview {
    pub fn scroll(&mut self, amount: isize) {
        self.top = self.top.saturating_add_signed(amount).min(
            self.lines
                .len()
                .saturating_sub(usize::from(self.inner.height)),
        );
    }
}

#[derive(Default)]
pub struct Popover {
    pub open: Option<Preview>,
    pending: Option<(AnchorHit, Instant)>,
    close_at: Option<Instant>,
}

impl Popover {
    pub fn clear(&mut self) -> bool {
        let visible = self.open.take().is_some();
        self.pending = None;
        self.close_at = None;
        visible
    }

    pub fn contains(&self, point: Position) -> bool {
        self.open
            .as_ref()
            .is_some_and(|preview| preview.area.contains(point))
    }

    pub fn pointer(&mut self, hit: Option<AnchorHit>, in_preview: bool, now: Instant) -> bool {
        if in_preview {
            self.close_at = None;
            return false;
        }
        if let Some(hit) = hit {
            if self
                .open
                .as_ref()
                .is_some_and(|preview| preview.anchor.same_anchor(&hit))
            {
                self.close_at = None;
                return false;
            }
            if self
                .pending
                .as_ref()
                .is_some_and(|(anchor, _)| anchor.same_anchor(&hit))
            {
                return false;
            }
            let changed = self.clear();
            self.pending = Some((hit, now + OPEN_DELAY));
            changed
        } else {
            self.pending = None;
            if self.open.is_some() && self.close_at.is_none() {
                self.close_at = Some(now + CLOSE_DELAY);
            }
            false
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.pending
            .as_ref()
            .map(|(_, time)| *time)
            .or(self.close_at)
    }

    pub fn advance(&mut self, now: Instant, thread: &ThreadView, content: Rect) -> bool {
        if self.close_at.is_some_and(|deadline| deadline <= now) {
            return self.clear();
        }
        if !self
            .pending
            .as_ref()
            .is_some_and(|(_, deadline)| *deadline <= now)
        {
            return false;
        }
        let (anchor, _) = self.pending.take().unwrap();
        let Some(&index) = thread.post_index.get(&anchor.target) else {
            return false;
        };
        let Some(mut area) = placement(content, anchor.area) else {
            return false;
        };
        let mut lines = render_post(
            &thread.snapshot.posts[index],
            area.width - 2,
            &thread.post_index,
        );
        // The thread separator is useful in the main view, but not inside a preview.
        while lines.last().is_some_and(|line| line.text.width() == 0) {
            lines.pop();
        }
        let height = area.height.min(
            u16::try_from(lines.len())
                .unwrap_or(u16::MAX)
                .saturating_add(2),
        );
        if area.y < anchor.area.y {
            area.y += area.height - height;
        }
        area.height = height;
        let inner = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);
        self.open = Some(Preview {
            anchor,
            area,
            inner,
            lines,
            top: 0,
        });
        true
    }
}

fn placement(content: Rect, anchor: Rect) -> Option<Rect> {
    let width = content.width.min(MAX_WIDTH);
    let desired_height = content.height / 2;
    if width < 4 || desired_height < 3 || !content.contains((anchor.x, anchor.y).into()) {
        return None;
    }
    let below = content.bottom().saturating_sub(anchor.bottom());
    let above = anchor.y.saturating_sub(content.y);
    let use_below = below >= desired_height || (above < desired_height && below >= above);
    let height = desired_height.min(if use_below { below } else { above });
    if height < 3 {
        return None;
    }
    Some(Rect::new(
        anchor.x.min(content.right() - width).max(content.x),
        if use_below {
            anchor.bottom()
        } else {
            anchor.y - height
        },
        width,
        height,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::viewer::{ReadingPosition, ThreadSnapshot, ViewerPost};

    fn thread() -> ThreadView {
        ThreadView::new(
            "test.dat".into(),
            ThreadSnapshot {
                title: String::new(),
                posts: vec![ViewerPost {
                    number: 3,
                    name: String::new(),
                    datetime: String::new(),
                    id: String::new(),
                    body: "本文\n".repeat(50),
                }],
            },
            ReadingPosition::default(),
        )
    }

    fn hit() -> AnchorHit {
        AnchorHit {
            source_post: 10,
            source_offset: 20,
            target: 3,
            area: Rect::new(8, 3, 3, 1),
        }
    }

    #[test]
    fn deadlines_cancel_and_crossing_into_preview_keeps_it_open() {
        let now = Instant::now();
        let content = Rect::new(0, 1, 80, 24);
        let thread = thread();
        let mut state = Popover::default();
        state.pointer(Some(hit()), false, now);
        assert!(!state.advance(now, &thread, content));
        state.pointer(None, false, now);
        assert_eq!(state.deadline(), None);
        state.pointer(Some(hit()), false, now);
        let mut fragment = hit();
        fragment.area.y += 1;
        state.pointer(Some(fragment), false, now + Duration::from_millis(100));
        assert_eq!(state.deadline(), Some(now + OPEN_DELAY));
        assert!(state.advance(now + OPEN_DELAY, &thread, content));
        state.pointer(None, false, now + OPEN_DELAY);
        state.pointer(None, true, now + OPEN_DELAY + Duration::from_millis(100));
        assert_eq!(state.deadline(), None);
        assert!(state.open.is_some());
        let preview = state.open.as_mut().unwrap();
        preview.scroll(isize::MAX);
        assert_eq!(
            preview.top,
            preview.lines.len() - usize::from(preview.inner.height)
        );
        preview.scroll(isize::MIN);
        assert_eq!(preview.top, 0);
        state.pointer(None, false, now + OPEN_DELAY);
        assert!(state.advance(now + OPEN_DELAY + CLOSE_DELAY, &thread, content));
        assert!(state.open.is_none());
    }

    #[test]
    fn placement_stays_inside_content_and_does_not_cover_anchor() {
        let content = Rect::new(2, 1, 80, 24);
        for y in 1..25 {
            let anchor = Rect::new(79, y, 3, 1);
            let popup = placement(content, anchor).unwrap();
            assert_eq!(popup.intersection(content), popup);
            assert!(!popup.intersects(anchor));
        }
        assert!(placement(Rect::new(0, 0, 2, 2), Rect::new(0, 0, 1, 1)).is_none());
    }
}
