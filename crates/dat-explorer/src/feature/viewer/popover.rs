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

//! Nested response previews, ordered back to front. The terminal loop drives deadlines.

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
    pub open: Vec<Preview>,
    pending: Option<(usize, AnchorHit, Instant)>,
    close_at: Option<(usize, Instant)>,
}

impl Popover {
    pub fn clear(&mut self) -> bool {
        self.truncate(0)
    }

    fn truncate(&mut self, keep: usize) -> bool {
        let changed = self.open.len() > keep;
        self.open.truncate(keep);
        self.pending = None;
        self.close_at = None;
        changed
    }

    pub fn close_deepest(&mut self) -> bool {
        self.truncate(self.open.len().saturating_sub(1))
    }

    pub fn layer_at(&self, point: Position) -> Option<usize> {
        self.open
            .iter()
            .rposition(|preview| preview.area.contains(point))
    }

    pub fn scroll(&mut self, layer: usize, amount: isize) {
        // Descendant anchor coordinates become invalid when their parent scrolls.
        self.truncate(layer + 1);
        self.open[layer].scroll(amount);
    }

    /// `layer` identifies the frontmost preview under the pointer, or the main view.
    pub fn pointer(&mut self, hit: Option<AnchorHit>, layer: Option<usize>, now: Instant) -> bool {
        let depth = layer.map_or(0, |index| index + 1);
        if let Some(hit) = hit {
            if self
                .open
                .get(depth)
                .is_some_and(|preview| preview.anchor.same_anchor(&hit))
            {
                self.pending = None;
                self.schedule_close(depth + 1, now);
                return false;
            }
            // A reference back to an ancestor never creates another copy of that response.
            if self.open[..depth]
                .iter()
                .any(|preview| preview.anchor.target == hit.target)
            {
                self.pending = None;
                self.schedule_close(depth, now);
                return false;
            }
            if let Some((pending_depth, anchor, _)) = &mut self.pending
                && *pending_depth == depth
                && anchor.same_anchor(&hit)
            {
                // Keep the deadline, but place beside the currently hovered wrapped fragment.
                anchor.area = hit.area;
                return false;
            }
            let changed = self.truncate(depth);
            self.pending = Some((depth, hit, now + OPEN_DELAY));
            changed
        } else {
            self.pending = None;
            self.schedule_close(depth, now);
            false
        }
    }

    fn schedule_close(&mut self, keep: usize, now: Instant) {
        if keep >= self.open.len() {
            self.close_at = None;
        } else if self.close_at.is_none_or(|(previous, _)| previous != keep) {
            self.close_at = Some((keep, now + CLOSE_DELAY));
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.pending
            .as_ref()
            .map(|(_, _, time)| *time)
            .or(self.close_at.map(|(_, time)| time))
    }

    pub fn advance(&mut self, now: Instant, thread: &ThreadView, content: Rect) -> bool {
        if let Some((keep, deadline)) = self.close_at
            && deadline <= now
        {
            return self.truncate(keep);
        }
        if !self
            .pending
            .as_ref()
            .is_some_and(|(_, _, deadline)| *deadline <= now)
        {
            return false;
        }
        let (depth, anchor, _) = self.pending.take().unwrap();
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
        self.open.truncate(depth);
        self.open.push(Preview {
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
                posts: [3, 4, 5, 6]
                    .into_iter()
                    .map(|number| ViewerPost {
                        number,
                        name: String::new(),
                        datetime: String::new(),
                        id: String::new(),
                        body: "本文\n".repeat(50),
                    })
                    .collect(),
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
        state.pointer(Some(hit()), None, now);
        assert!(!state.advance(now, &thread, content));
        state.pointer(None, None, now);
        assert_eq!(state.deadline(), None);
        state.pointer(Some(hit()), None, now);
        let mut fragment = hit();
        fragment.area.y += 1;
        state.pointer(Some(fragment), None, now + Duration::from_millis(100));
        assert_eq!(state.deadline(), Some(now + OPEN_DELAY));
        assert!(state.advance(now + OPEN_DELAY, &thread, content));
        assert_eq!(state.open[0].anchor.area.y, hit().area.y + 1);
        assert!(!state.open[0].area.intersects(state.open[0].anchor.area));
        state.pointer(None, None, now + OPEN_DELAY);
        state.pointer(None, Some(0), now + OPEN_DELAY + Duration::from_millis(100));
        assert_eq!(state.deadline(), None);
        assert_eq!(state.open.len(), 1);
        let preview = state.open.last_mut().unwrap();
        preview.scroll(isize::MAX);
        assert_eq!(
            preview.top,
            preview.lines.len() - usize::from(preview.inner.height)
        );
        preview.scroll(isize::MIN);
        assert_eq!(preview.top, 0);
        state.pointer(None, None, now + OPEN_DELAY);
        assert!(state.advance(now + OPEN_DELAY + CLOSE_DELAY, &thread, content));
        assert!(state.open.is_empty());
    }

    #[test]
    fn nested_deadlines_preserve_ancestors_and_close_only_the_departed_branch() {
        let mut now = Instant::now();
        let thread = thread();
        let content = Rect::new(0, 1, 100, 32);
        let mut state = Popover::default();
        let mut anchors = Vec::new();
        for depth in 0..3 {
            let anchor = AnchorHit {
                source_post: if depth == 0 { 10 } else { depth + 2 },
                target: depth + 3,
                area: Rect::new(8 + depth as u16, 3 + depth as u16 * 3, 3, 1),
                ..hit()
            };
            state.pointer(Some(anchor.clone()), depth.checked_sub(1), now);
            now += OPEN_DELAY;
            assert!(state.advance(now, &thread, content));
            assert_eq!(state.open.len(), depth + 1);
            anchors.push(anchor);
        }
        assert!(state.open[0].area.intersects(state.open[1].area));
        assert!(!state.open[1].area.intersects(anchors[1].area));
        let point = (state.open[2].area.x, state.open[2].area.y).into();
        assert_eq!(state.layer_at(point), Some(2));

        // Returning to the child-opening anchor keeps the child, but schedules its child away.
        state.pointer(Some(anchors[1].clone()), Some(0), now);
        assert_eq!(state.deadline(), Some(now + CLOSE_DELAY));
        state.pointer(None, Some(2), now + Duration::from_millis(100));
        assert_eq!(state.deadline(), None); // Crossing back into the grandchild cancels closing.
        state.pointer(Some(anchors[1].clone()), Some(0), now);
        now += CLOSE_DELAY;
        assert!(state.advance(now, &thread, content));
        assert_eq!(state.open.len(), 2);

        state.pointer(None, Some(0), now);
        now += CLOSE_DELAY;
        assert!(state.advance(now, &thread, content));
        assert_eq!(state.open.len(), 1);
        state.pointer(None, None, now);
        now += CLOSE_DELAY;
        assert!(state.advance(now, &thread, content));
        assert!(state.open.is_empty());
    }

    #[test]
    fn siblings_pending_cancellation_cycles_and_escape() {
        let mut now = Instant::now();
        let content = Rect::new(0, 1, 80, 24);
        let thread = thread();
        let mut state = Popover::default();
        state.pointer(Some(hit()), None, now);
        now += OPEN_DELAY;
        state.advance(now, &thread, content);
        let child = AnchorHit {
            source_post: 3,
            target: 4,
            area: Rect::new(9, 6, 3, 1),
            ..hit()
        };
        state.pointer(Some(child.clone()), Some(0), now);
        state.pointer(None, Some(0), now);
        assert_eq!(state.deadline(), None);
        state.pointer(Some(child.clone()), Some(0), now);
        now += OPEN_DELAY;
        state.advance(now, &thread, content);
        assert_eq!(state.open.len(), 2);
        // A -> B -> A and B -> B must never grow the stack.
        for target in [3, 4] {
            state.pointer(
                Some(AnchorHit {
                    source_post: 4,
                    target,
                    ..child.clone()
                }),
                Some(1),
                now,
            );
            assert_eq!(state.deadline(), None);
            assert_eq!(state.open.len(), 2);
        }
        let sibling = AnchorHit {
            source_offset: 99,
            target: 5,
            ..child
        };
        assert!(state.pointer(Some(sibling), Some(0), now));
        assert_eq!(state.open.len(), 1);
        now += OPEN_DELAY;
        state.advance(now, &thread, content);
        assert_eq!(state.open[1].anchor.target, 5);
        assert!(state.close_deepest());
        assert_eq!(state.open.len(), 1);
        assert_eq!(state.deadline(), None);
        assert!(state.close_deepest());
        assert!(!state.close_deepest());
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
