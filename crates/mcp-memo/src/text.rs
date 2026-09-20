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

use rust_myscript::prelude::*;

pub(crate) fn replace_memo_text(
    content: &str,
    old: &str,
    new: &str,
    key: &str,
) -> Fallible<String> {
    if old.is_empty() {
        bail!("old must not be empty");
    }
    let first = match content.find(old) {
        Some(p) => p,
        None => {
            bail!("old text not found in memo '{key}'");
        }
    };
    // Check for a second occurrence (including overlapping ones) by advancing only one char.
    let mut second_search_start = first;
    if let Some((delta, _)) = content[second_search_start..].char_indices().nth(1) {
        second_search_start += delta;
    } else {
        second_search_start = content.len();
    }
    if second_search_start < content.len() && content[second_search_start..].contains(old) {
        bail!("old text occurs multiple times in memo '{key}'");
    }

    let mut new_content = content.to_string();
    new_content.replace_range(first..first + old.len(), new);
    Ok(new_content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_memo_text_should_replace_single_occurrence() {
        for (content, old, new, expected) in [
            ("hello world", "world", "rust", "hello rust"),
            ("日本語のメモ", "日本語", "英語", "英語のメモ"),
            (
                "前文：日本語のメモ。",
                "日本語",
                "English",
                "前文：Englishのメモ。",
            ),
            ("末尾あ", "あ", "い", "末尾い"),
            ("あ", "あ", "", ""),
            ("ああああ", "ああああ", "X", "X"),
            ("hello", "hello", "hello", "hello"),
            (
                "header\nbody\nfooter",
                "body\nfooter",
                "newbody\nnewfooter",
                "header\nnewbody\nnewfooter",
            ),
            (
                "前\n本文\n後",
                "\n本文\n",
                "\n新しい本文\n",
                "前\n新しい本文\n後",
            ),
        ] {
            assert_eq!(
                replace_memo_text(content, old, new, "doc").unwrap(),
                expected
            );
        }
    }

    #[test]
    fn replace_memo_text_should_reject_empty_or_missing_old() {
        for (content, old, expected) in [
            ("", "", "old must not be empty"),
            ("日本語", "", "old must not be empty"),
            ("", "text", "old text not found in memo 'doc'"),
            ("hello", "world", "old text not found in memo 'doc'"),
            ("日本語", "英語", "old text not found in memo 'doc'"),
        ] {
            assert_eq!(
                replace_memo_text(content, old, "replacement", "doc")
                    .unwrap_err()
                    .to_string(),
                expected
            );
        }
    }

    #[test]
    fn replace_memo_text_should_reject_duplicate_and_overlapping_matches() {
        for (content, old) in [
            ("ab ab ab", "ab"),
            ("a\nb\na\nb", "a\nb"),
            ("日本語と日本語", "日本語"),
            ("aaaa", "aaa"),
            ("ああああ", "あああ"),
            ("前：ああああ", "あああ"),
            ("あ\nあ\nあ", "あ\nあ"),
        ] {
            assert_eq!(
                replace_memo_text(content, old, "replacement", "doc")
                    .unwrap_err()
                    .to_string(),
                "old text occurs multiple times in memo 'doc'"
            );
        }
    }
}
