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

//! CP932 decoding and parsing of subject.txt, independent of its source.

use crate::model::fetch::SubjectThread;
use regex::Regex;
use rust_myscript::prelude::*;
use std::sync::LazyLock;

/// Thread key of a `subject.txt` line, e.g. `1234567890.dat`.
///
/// `[0-9]` is used instead of `\d` on purpose: the `regex` crate's `\d` is Unicode-aware and
/// would also accept full-width digits such as `１２３`.
static RE_THREAD_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([0-9]+)\.dat$").unwrap());

/// Trailing res count of a `subject.txt` line, e.g. `  (634)`.
///
/// Notes:
/// - The `$` anchor is mandatory. Some real titles end with `(数字)` themselves
///   (`テストスレッド★633 (21146)  (36)`), so only the right-most parenthesised number is the res
///   count. Searching for `(` from the left, or dropping `$`, breaks those lines.
/// - `[0-9]` is used instead of `\d` for the same reason as [`RE_THREAD_KEY`].
/// - The separator is `\s+` rather than a fixed `  `. Measured over 52 boards / 20,382 lines the
///   separator was `"  "`, `" "`, `" \t "`, `"\t "` and full-width variants; a fixed `  `
///   separator failed on 48.5% of the lines.
/// - The `regex` crate's `\s` is Unicode-aware and therefore matches U+3000 (full-width space).
///   As a result a full-width space at the *head* of a title is kept while one at the *tail* is
///   consumed as part of the separator. The asymmetry is harmless because a separator only ever
///   exists on the tail side. Narrowing `\s+` to `[ \t]+` would break the 28+ observed lines that
///   use a full-width separator, so it stays `\s+`.
static RE_RES_COUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+\(([0-9]+)\)\s*$").unwrap());

/// Decimal / hexadecimal numeric references. Named entities are intentionally out of scope.
static RE_NCR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&#(?:[xX]([0-9a-fA-F]+)|([0-9]+));").unwrap());

/// Decodes CP932 strictly; replacing broken bytes with U+FFFD would hide format changes.
pub(super) fn decode_cp932(bytes: &[u8]) -> Fallible<String> {
    match encoding_rs::SHIFT_JIS.decode_without_bom_handling_and_without_replacement(bytes) {
        Some(text) => Ok(text.into_owned()),
        None => bail!("CP932 として解釈できないバイト列が含まれています"),
    }
}

/// Parses the whole body. Only completely empty lines are skipped; whitespace-only lines
/// are errors, preserving the distinction between content and title whitespace.
pub(super) fn parse_subject(text: &str) -> Fallible<Vec<SubjectThread>> {
    let mut threads = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let line_no = index + 1;
        threads.push(parse_line(line).with_context(|| format!("{line_no} 行目"))?);
    }
    Ok(threads)
}

fn parse_line(line: &str) -> Fallible<SubjectThread> {
    let Some((key, rest)) = line.split_once("<>") else {
        bail!("セパレーター \"<>\" が見つかりません");
    };
    let Some(caps) = RE_THREAD_KEY.captures(key) else {
        bail!("スレッドキーの形式が不正です: {key:?}");
    };
    let thread_id = caps[1].to_owned();
    let Some(caps) = RE_RES_COUNT.captures(rest) else {
        bail!("レス数が見つかりません: {rest:?}");
    };
    let res_count = caps[1]
        .parse::<u64>()
        .with_context(|| format!("レス数を数値に変換できません: {:?}", &caps[1]))?;
    let separator_start = caps.get(0).expect("group 0 always exists").start();
    let title = &rest[..separator_start];
    if title.is_empty() {
        bail!("スレッドタイトルが空です");
    }
    Ok(SubjectThread {
        thread_id,
        title: restore_ncr(title),
        res_count,
    })
}

/// Invalid scalar values and unterminated references are left unchanged, as in
/// the dat body mapper.
fn restore_ncr(s: &str) -> String {
    RE_NCR
        .replace_all(s, |caps: &regex::Captures| {
            let code_point = match caps.get(1) {
                Some(hex) => u32::from_str_radix(hex.as_str(), 16).ok(),
                None => caps[2].parse::<u32>().ok(),
            };
            code_point
                .and_then(char::from_u32)
                .map_or_else(|| caps[0].to_owned(), |c| c.to_string())
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(thread_id: &str, title: &str, res_count: u64) -> SubjectThread {
        SubjectThread {
            thread_id: thread_id.to_owned(),
            title: title.to_owned(),
            res_count,
        }
    }

    #[test]
    fn parse_subject_basic() {
        let actual = parse_subject("1234567890.dat<>テストスレッド★630  (649)").unwrap();
        assert_eq!(
            actual,
            vec![thread("1234567890", "テストスレッド★630", 649)]
        );
    }

    #[test]
    fn parse_subject_single_space_separator() {
        let actual = parse_subject("1234567890.dat<>タイトル (1)").unwrap();
        assert_eq!(actual, vec![thread("1234567890", "タイトル", 1)]);
    }

    #[test]
    fn parse_subject_many_space_separator() {
        let actual = parse_subject("1234567890.dat<>タイトル     (12)").unwrap();
        assert_eq!(actual, vec![thread("1234567890", "タイトル", 12)]);
    }

    #[test]
    fn parse_subject_tab_separator() {
        let actual = parse_subject("1.dat<>A \t (2)\n2.dat<>B\t (3)\n3.dat<>C\t(4)").unwrap();
        assert_eq!(
            actual,
            vec![
                thread("1", "A", 2),
                thread("2", "B", 3),
                thread("3", "C", 4)
            ]
        );
    }

    #[test]
    fn parse_subject_fullwidth_separator() {
        let actual = parse_subject("1234567890.dat<>タイトル　  (5)").unwrap();
        assert_eq!(actual, vec![thread("1234567890", "タイトル", 5)]);
    }

    #[test]
    fn parse_subject_keeps_leading_half_and_full_width_space() {
        let actual =
            parse_subject("1234567894.dat<> ★ テスト告知スレッド (2)\n1.dat<>　全角先頭  (3)")
                .unwrap();
        assert_eq!(
            actual,
            vec![
                thread("1234567894", " ★ テスト告知スレッド", 2),
                thread("1", "　全角先頭", 3)
            ]
        );
    }

    #[test]
    fn parse_subject_consumes_only_trailing_separator_space() {
        let actual = parse_subject("1.dat<>　タイトル　  (10)").unwrap();
        assert_eq!(actual, vec![thread("1", "　タイトル", 10)]);
    }

    #[test]
    fn parse_subject_title_contains_parentheses() {
        let actual = parse_subject("1.dat<>雑談(実況)スレ  (42)").unwrap();
        assert_eq!(actual, vec![thread("1", "雑談(実況)スレ", 42)]);
    }

    #[test]
    fn parse_subject_title_ends_with_parenthesised_number() {
        let actual = parse_subject(
            "1.dat<>テストスレッド (11)  (307)\n2.dat<>テストスレッド★633 (21146)  (36)",
        )
        .unwrap();
        assert_eq!(
            actual,
            vec![
                thread("1", "テストスレッド (11)", 307),
                thread("2", "テストスレッド★633 (21146)", 36)
            ]
        );
    }

    #[test]
    fn parse_subject_keeps_be_id_in_title() {
        let actual =
            parse_subject("1234567890.dat<>タイトル [BE:1234567890-2BP(1000)]  (634)").unwrap();
        assert_eq!(
            actual,
            vec![thread(
                "1234567890",
                "タイトル [BE:1234567890-2BP(1000)]",
                634
            )]
        );
    }

    #[test]
    fn parse_subject_keeps_leading_zero_thread_id() {
        let actual = parse_subject("0000000000.dat<>タイトル  (7)").unwrap();
        assert_eq!(actual, vec![thread("0000000000", "タイトル", 7)]);
    }

    #[test]
    fn parse_subject_lf() {
        let actual = parse_subject("1.dat<>A  (1)\n2.dat<>B  (2)\n").unwrap();
        assert_eq!(actual, vec![thread("1", "A", 1), thread("2", "B", 2)]);
    }

    #[test]
    fn parse_subject_crlf() {
        let actual = parse_subject("1.dat<>A  (1)\r\n2.dat<>B  (2)\r\n").unwrap();
        assert_eq!(actual, vec![thread("1", "A", 1), thread("2", "B", 2)]);
    }

    #[test]
    fn parse_subject_without_trailing_newline() {
        let actual = parse_subject("1.dat<>A  (1)\n2.dat<>B  (2)").unwrap();
        assert_eq!(actual, vec![thread("1", "A", 1), thread("2", "B", 2)]);
    }

    #[test]
    fn parse_subject_keeps_order_and_duplicates() {
        let actual = parse_subject("3.dat<>C  (3)\n1.dat<>A  (1)\n3.dat<>C  (3)").unwrap();
        assert_eq!(
            actual,
            vec![
                thread("3", "C", 3),
                thread("1", "A", 1),
                thread("3", "C", 3)
            ]
        );
    }

    #[test]
    fn parse_subject_zero_res_count() {
        let actual = parse_subject("1.dat<>タイトル  (0)").unwrap();
        assert_eq!(actual, vec![thread("1", "タイトル", 0)]);
    }

    #[test]
    fn parse_subject_max_u64_res_count() {
        let actual = parse_subject("1.dat<>タイトル  (18446744073709551615)").unwrap();
        assert_eq!(actual, vec![thread("1", "タイトル", u64::MAX)]);
    }

    #[test]
    fn parse_subject_empty_body() {
        assert_eq!(parse_subject("").unwrap(), vec![]);
        assert_eq!(parse_subject("\n").unwrap(), vec![]);
        assert_eq!(parse_subject("\r\n").unwrap(), vec![]);
    }

    #[test]
    fn parse_subject_ignores_empty_lines_only() {
        assert_eq!(parse_subject("\n\n\n").unwrap(), vec![]);
        let actual = parse_subject("\n1.dat<>A  (1)\n\n2.dat<>B  (2)\n\n").unwrap();
        assert_eq!(actual, vec![thread("1", "A", 1), thread("2", "B", 2)]);
    }

    fn parse_err(text: &str) -> String {
        format!("{:#}", parse_subject(text).unwrap_err())
    }

    #[test]
    fn parse_subject_without_separator() {
        let actual = parse_err("1.dat 単なる文字列 (1)");
        assert!(actual.contains("1 行目"), "{actual}");
        assert!(actual.contains("<>"), "{actual}");
    }

    #[test]
    fn parse_subject_without_dat_suffix() {
        let actual = parse_err("1.dat<>A  (1)\n1234567890<>B  (2)");
        assert!(actual.contains("2 行目"), "{actual}");
    }

    #[test]
    fn parse_subject_non_ascii_digit_thread_id() {
        let actual = parse_err("１２３.dat<>A  (1)");
        assert!(actual.contains("1 行目"), "{actual}");
        let actual = parse_err("abc.dat<>A  (1)");
        assert!(actual.contains("1 行目"), "{actual}");
    }

    #[test]
    fn parse_subject_without_res_count() {
        let actual = parse_err("1.dat<>タイトルのみ");
        assert!(actual.contains("1 行目"), "{actual}");
        let actual = parse_err("1.dat<>タイトル  (12)x");
        assert!(actual.contains("1 行目"), "{actual}");
    }

    #[test]
    fn parse_subject_non_ascii_digit_res_count() {
        let actual = parse_err("1.dat<>タイトル  (１２)");
        assert!(actual.contains("1 行目"), "{actual}");
        let actual = parse_err("1.dat<>タイトル  (abc)");
        assert!(actual.contains("1 行目"), "{actual}");
    }

    #[test]
    fn parse_subject_res_count_overflow() {
        let actual = parse_err("1.dat<>タイトル  (18446744073709551616)");
        assert!(actual.contains("1 行目"), "{actual}");
        assert!(actual.contains("レス数"), "{actual}");
    }

    #[test]
    fn parse_subject_empty_title() {
        let actual = parse_err("1.dat<> (1)");
        assert!(actual.contains("1 行目"), "{actual}");
        assert!(actual.contains("タイトル"), "{actual}");
    }

    #[test]
    fn parse_subject_whitespace_only_line() {
        let actual = parse_err("1.dat<>A  (1)\n   \n2.dat<>B  (2)");
        assert!(actual.contains("2 行目"), "{actual}");
    }

    #[test]
    fn parse_subject_error_line_number_counts_empty_lines() {
        let actual = parse_err("\n\n1.dat<>A  (1)\n\nbroken");
        assert!(actual.contains("5 行目"), "{actual}");
    }

    #[test]
    fn restore_ncr_decimal() {
        assert_eq!(
            restore_ncr("&#129402;絵文字テストスレッド"),
            "🥺絵文字テストスレッド"
        );
    }

    #[test]
    fn restore_ncr_hex_lowercase() {
        assert_eq!(restore_ncr("&#x1F97A;"), "🥺");
        assert_eq!(restore_ncr("&#x1f97a;"), "🥺");
    }

    #[test]
    fn restore_ncr_hex_uppercase_marker() {
        assert_eq!(restore_ncr("&#X1F97A;"), "🥺");
    }

    #[test]
    fn restore_ncr_supplementary_plane() {
        assert_eq!(restore_ncr("&#131083;"), "\u{2000B}");
    }

    #[test]
    fn restore_ncr_multiple_mixed() {
        assert_eq!(
            restore_ncr("&#129402;テスト&#x1F97A;スレッド&#65;"),
            "🥺テスト🥺スレッドA"
        );
    }

    #[test]
    fn restore_ncr_keeps_surrogate() {
        assert_eq!(restore_ncr("&#55296;"), "&#55296;");
        assert_eq!(restore_ncr("&#xD800;"), "&#xD800;");
    }

    #[test]
    fn restore_ncr_keeps_out_of_range() {
        assert_eq!(restore_ncr("&#1114112;"), "&#1114112;");
        assert_eq!(restore_ncr("&#x110000;"), "&#x110000;");
        assert_eq!(restore_ncr("&#99999999999999;"), "&#99999999999999;");
        assert_eq!(restore_ncr("&#xFFFFFFFFFF;"), "&#xFFFFFFFFFF;");
    }

    #[test]
    fn restore_ncr_keeps_unterminated_and_invalid() {
        assert_eq!(restore_ncr("&#129402"), "&#129402");
        assert_eq!(restore_ncr("&#;"), "&#;");
        assert_eq!(restore_ncr("&#x;"), "&#x;");
        assert_eq!(restore_ncr("&#abc;"), "&#abc;");
        assert_eq!(restore_ncr("A&amp;B"), "A&amp;B");
    }

    #[test]
    fn parse_subject_restores_ncr_in_title() {
        let actual =
            parse_subject("1234567892.dat<>&#129402;絵文字テストスレッド&#129402;★631  (398)")
                .unwrap();
        assert_eq!(
            actual,
            vec![thread("1234567892", "🥺絵文字テストスレッド🥺★631", 398)]
        );
    }

    #[test]
    fn decode_cp932_ascii() {
        assert_eq!(
            decode_cp932(b"1.dat<>title  (1)\n").unwrap(),
            "1.dat<>title  (1)\n"
        );
    }

    #[test]
    fn decode_cp932_japanese() {
        let (bytes, _, unmappable) = encoding_rs::SHIFT_JIS.encode("テストスレッド★630");
        assert!(!unmappable);
        assert_eq!(decode_cp932(&bytes).unwrap(), "テストスレッド★630");
    }

    #[test]
    fn decode_cp932_rejects_invalid_bytes() {
        assert!(decode_cp932(&[0x81, 0x20]).is_err());
        assert!(decode_cp932(&[0x82, 0xA0, 0x81]).is_err());
    }

    #[test]
    fn decode_cp932_empty() {
        assert_eq!(decode_cp932(b"").unwrap(), "");
    }
}
