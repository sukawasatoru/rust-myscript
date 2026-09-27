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

//! Links in decoded post bodies, independent of their source format.

use regex::Regex;
use std::sync::LazyLock;

static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"h?ttps?://[a-zA-Z0-9\-._~:/?#\[\]@!$&'()*+,;=%]+").unwrap());

/// 5ch infrastructure and ancillary service hosts to exclude from URL extraction.
/// - jump5.ch: 5ch's redirect proxy (wraps external links)
/// - 5ch.io/test: read.cgi thread links (internal navigation, not user content)
/// - seesaawiki, donguri, majinai: board-associated services (wikis, acorn system, etc.)
static EXCLUDED_HOSTS: &[&str] = &[
    "jump5.ch",
    "5ch.io/test",
    "seesaawiki",
    "donguri",
    "majinai",
];

/// Extracts URLs, restoring the leading `h` omitted by the `ttp(s)://` convention.
pub fn extract_urls(text: &str) -> Vec<String> {
    URL_RE
        .find_iter(text)
        .map(|m| {
            let url = m.as_str();
            if url.starts_with("ttp") && !url.starts_with("http") {
                format!("h{url}")
            } else {
                url.to_string()
            }
        })
        .filter(|u| !is_excluded_url(u))
        .collect()
}

pub fn is_excluded_url(url: &str) -> bool {
    EXCLUDED_HOSTS.iter().any(|host| url.contains(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_urls_basic() {
        let urls = extract_urls("check https://example.com/test.jpg and http://foo.bar/baz");
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("example.com"));
    }

    #[test]
    fn extract_urls_excludes_jump5ch() {
        let urls = extract_urls("link http://jump5.ch/?https://real.url/test");
        assert!(urls.is_empty());
    }

    #[test]
    fn extract_urls_ttp_prefix() {
        let urls = extract_urls("check ttp://example.com/test and ttps://example.com/secure");
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "http://example.com/test");
        assert_eq!(urls[1], "https://example.com/secure");
    }

    #[test]
    fn extract_urls_mixed_http_and_ttp() {
        let urls = extract_urls("https://normal.com ttp://legacy.com");
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "https://normal.com");
        assert_eq!(urls[1], "http://legacy.com");
    }
}
