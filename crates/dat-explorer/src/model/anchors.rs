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

//! Response references in decoded text, independent of display and thread contents.

use regex::Regex;
use std::ops::Range;
use std::sync::LazyLock;

static ANCHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r">>[0-9]+(?:[-,][0-9]*)*").unwrap());

#[derive(Debug, PartialEq, Eq)]
pub struct Anchor {
    pub range: Range<usize>,
    pub post_number: usize,
}

pub fn find_anchors(text: &str) -> Vec<Anchor> {
    ANCHOR
        .find_iter(text)
        .filter_map(|matched| {
            // Do not treat a prefix of a range/list or an extra '>' as a reference.
            if text[..matched.start()].ends_with('>') {
                return None;
            }
            let number = matched.as_str()[2..].parse::<usize>().ok()?;
            (number != 0).then_some(Anchor {
                range: matched.range(),
                post_number: number,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoded_single_references_only() {
        let text = "日本語 >>12 >>003 >>0 >>1-3 >>2,4 >>>5 >>9999999999999999999999999 >>６ >>7";
        let anchors = find_anchors(text);
        assert_eq!(
            anchors.iter().map(|a| a.post_number).collect::<Vec<_>>(),
            [12, 3, 7]
        );
        assert_eq!(&text[anchors[0].range.clone()], ">>12");
    }
}
