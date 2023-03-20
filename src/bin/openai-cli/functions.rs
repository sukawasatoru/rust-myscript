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

use crate::data::repository::{GetPreferencesRepository, PreferencesRepository};
use reqwest::header::{self, HeaderMap, HeaderValue};
use rust_myscript::prelude::*;

pub fn prepare_headers<Ctx>(
    context: &Ctx,
    arg_organization_id: Option<String>,
    arg_api_key: Option<String>,
) -> Fallible<HeaderMap>
where
    Ctx: GetPreferencesRepository,
{
    let settings = context.get_prefs_repo().load_settings()?;

    let mut default_headers = HeaderMap::new();

    let api_key = match arg_api_key.or(settings.api_key) {
        Some(data) => data,
        None => bail!("need api_key"),
    };

    let mut api_key = format!("Bearer {api_key}").parse::<HeaderValue>()?;
    api_key.set_sensitive(true);
    default_headers.insert(header::AUTHORIZATION, api_key);

    if let Some(organization_id) = arg_organization_id.or(settings.organization_id) {
        default_headers.insert("OpenAI-Organization", organization_id.parse()?);
    }

    Ok(default_headers)
}

pub fn print_stdin_help() {
    #[cfg(target_os = "windows")]
    {
        eprintln!("Please input message and Ctrl+Z");
    }

    #[cfg(not(target_os = "windows"))]
    {
        eprintln!("Please input message and Ctrl+D");
    }
}
