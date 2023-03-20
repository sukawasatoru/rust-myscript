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
use crate::SettingsKey;
use rust_myscript::prelude::*;

pub fn list_settings<Ctx>(context: Ctx) -> Fallible<()>
where
    Ctx: GetPreferencesRepository,
{
    let settings = context.get_prefs_repo().load_settings()?;

    print_setting(&SettingsKey::OrganizationId, &settings.organization_id);
    print_setting(&SettingsKey::ApiKey, &settings.api_key);

    Ok(())
}

pub fn get_setting<Ctx>(context: Ctx, key: &SettingsKey) -> Fallible<()>
where
    Ctx: GetPreferencesRepository,
{
    let settings = context.get_prefs_repo().load_settings()?;

    match *key {
        SettingsKey::OrganizationId => {
            print_setting(&SettingsKey::OrganizationId, &settings.organization_id)
        }
        SettingsKey::ApiKey => print_setting(&SettingsKey::ApiKey, &settings.api_key),
    }

    Ok(())
}

pub fn set_setting<Ctx>(context: Ctx, key: &SettingsKey, value: String) -> Fallible<()>
where
    Ctx: GetPreferencesRepository,
{
    let repo = context.get_prefs_repo();
    let mut settings = repo.load_settings()?;

    match *key {
        SettingsKey::OrganizationId => {
            settings.organization_id = Some(value);
        }
        SettingsKey::ApiKey => {
            settings.api_key = Some(value);
        }
    }

    repo.save_settings(&settings)?;

    Ok(())
}

fn print_setting(key: &SettingsKey, value: &Option<String>) {
    println!(
        "{key}: {}",
        value.as_ref().map(|data| data.as_str()).unwrap_or("(none)"),
    );
}
