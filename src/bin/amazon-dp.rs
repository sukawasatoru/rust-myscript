/*
 * Copyright 2024 sukawasatoru
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

use clap::{Parser, ValueHint};
use rust_myscript::prelude::*;
use url::Url;

/// Create simple URL for amazon.
#[derive(Parser)]
struct Opt {
    /// Amazon URL.
    #[arg(value_hint = ValueHint::Url)]
    input: Url,
}

fn main() {
    let opt = Opt::parse();
    match create_short_url(&opt.input) {
        Ok(short_url) => println!("{short_url}"),
        Err(_) => println!("{}", opt.input),
    }
}

fn create_short_url(input: &Url) -> Fallible<Url> {
    let mut segments = match input.path_segments() {
        Some(data) => data,
        None => bail!("no base url"),
    };

    while let Some(segment) = segments.next() {
        if segment != "dp" {
            continue;
        }

        let id = match segments.next() {
            Some(data) => data,
            None => bail!("no dp id"),
        };

        let mut short_url = input.clone();
        short_url.set_path(&format!("/dp/{id}"));
        short_url.set_query(None);
        short_url.set_fragment(None);
        return Ok(short_url);
    }

    bail!("unsupported format")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn struct_opt() {
        Opt::command().debug_assert();
    }

    #[ignore]
    #[test]
    fn struct_opt_help() {
        Opt::command().print_help().unwrap();
    }

    #[test]
    fn struct_opt_url_format() {
        Opt::command().get_matches_from(["cmd-name", "https://example.com/bar"]);
    }

    #[test]
    fn struct_opt_data_uri_format() {
        Opt::command().get_matches_from(["cmd-name", "data:text/plain,HelloWorld"]);
    }

    #[test]
    fn create_short_url_data_uri() {
        let actual = create_short_url(&Url::parse("data:text/plain,HelloWorld").unwrap());
        assert!(actual.is_err());
    }

    #[test]
    fn create_short_url_empty_segments() {
        let actual = create_short_url(&Url::parse("https://www.amazon.co.jp").unwrap());
        assert!(actual.is_err());
    }

    #[test]
    fn create_short_url_short() {
        let actual =
            create_short_url(&Url::parse("https://www.amazon.co.jp/dp/12345").unwrap()).unwrap();
        assert_eq!(
            actual,
            Url::parse("https://www.amazon.co.jp/dp/12345").unwrap(),
        );
    }

    #[test]
    fn create_short_url_nintendo_switch() {
        let actual = create_short_url(&Url::parse("https://www.amazon.co.jp/Nintendo-Switch-Joy-ネオンブルー-ネオンレッド/dp/B0BM46DFH1/ref=sr_1_3?__mk_ja_JP=カタカナ&crid=2AQ07ADQ6ZENA&keywords=Nintendo+switch&qid=1705937708&sprefix=nintendo+switch%2Caps%2C183&sr=8-3").unwrap()).unwrap();
        assert_eq!(
            actual,
            Url::parse("https://www.amazon.co.jp/dp/B0BM46DFH1").unwrap(),
        );
    }

    #[test]
    fn create_short_url_nintendo_switch_with_fragment() {
        let actual = create_short_url(&Url::parse("https://www.amazon.co.jp/Nintendo-Switch-Joy-ネオンブルー-ネオンレッド/dp/B0BM46DFH1/ref=sr_1_3?__mk_ja_JP=カタカナ&crid=2AQ07ADQ6ZENA&keywords=Nintendo+switch&qid=1705937708&sprefix=nintendo+switch%2Caps%2C183&sr=8-3#detailBulletsWrapper_feature_div").unwrap()).unwrap();
        assert_eq!(
            actual,
            Url::parse("https://www.amazon.co.jp/dp/B0BM46DFH1").unwrap(),
        );
    }
}
