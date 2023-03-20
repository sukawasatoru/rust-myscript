/*
 * Copyright 2023 sukawasatoru
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

use crate::data::repository::GetPreferencesRepository;
use crate::functions::{prepare_headers, print_stdin_help};
use reqwest::blocking::Client;
use rust_myscript::prelude::*;
use serde::Serialize;
use serde_json::json;
use std::io::prelude::*;

pub fn edit_translate<Ctx>(
    context: Ctx,
    arg_organization_id: Option<String>,
    arg_api_key: Option<String>,
    target: String,
) -> Fallible<()>
where
    Ctx: GetPreferencesRepository,
{
    let default_headers = prepare_headers(&context, arg_organization_id, arg_api_key)?;

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(60 * 5))
        .default_headers(default_headers)
        .build()?;

    print_stdin_help();

    let mut read_buf = String::new();

    std::io::stdin().read_to_string(&mut read_buf)?;

    let content = read_buf.trim().to_owned();
    if content.is_empty() {
        return Ok(());
    }

    eprintln!("...");

    let body = json!({
        "model": EditModel::TextDavinchEdit001,
        "input": content,
        "instruction": format!("Translate to {target}"),
    });

    let ret = client
        .post("https://api.openai.com/v1/edits")
        .json(&body)
        .send()?
        .error_for_status()?
        .text()?;
    trace!(%ret);
    let ret = serde_json::from_str::<serde_json::Value>(&ret)?;
    let ret = ret["choices"][0]["text"]
        .as_str()
        .context("empty response text")?;
    eprintln!("{ret}");

    Ok(())
}

/// ref. https://platform.openai.com/docs/api-reference/edits/create
#[derive(Clone, Serialize)]
enum EditModel {
    #[serde(rename = "text-davinci-edit-001")]
    TextDavinchEdit001,

    #[allow(dead_code)]
    #[serde(rename = "code-davinci-edit-001")]
    CodeDavinciEdit001,
}
