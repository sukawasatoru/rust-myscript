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

#[derive(Debug, Clone, PartialEq)]
pub struct GadgetSnapshot {
    pub last_update: u32,
    pub disks: Vec<DiskEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiskEntry {
    pub model_serial: String,
    pub drive_letter: String,
    pub model: String,
    pub disk_size: String,
    pub temperature_celsius: Option<f64>,
    pub temperature_class: String,
    pub disk_status: u32,
}

pub fn parse_temperature(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    let mut end = 0;
    let mut started = false;
    for (i, c) in raw.char_indices() {
        if !started && c.is_whitespace() {
            continue;
        }
        if c.is_ascii_digit() || c == '.' || (!started && (c == '-' || c == '+')) {
            started = true;
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    if !started {
        return None;
    }
    let num: f64 = raw.get(..end)?.parse().ok()?;
    let rest = raw.get(end..)?.to_ascii_uppercase();
    if rest.contains('F') {
        Some((num - 32.0) * 5.0 / 9.0)
    } else if rest.contains('C') {
        Some(num)
    } else {
        None
    }
}

pub fn disk_status_name(status: u32) -> &'static str {
    match status {
        0 => "unknown",
        1 => "good",
        2 => "caution",
        3 => "bad",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_temperature_celsius() {
        assert_eq!(parse_temperature("45 °C"), Some(45.0));
        assert_eq!(parse_temperature(" 0 °C"), Some(0.0));
    }

    #[test]
    fn parse_temperature_fahrenheit() {
        let c = parse_temperature("113 °F").unwrap();
        assert!((c - 45.0).abs() < 1e-9);
    }

    #[test]
    fn parse_temperature_unknown() {
        assert_eq!(parse_temperature("-- °C"), None);
        assert_eq!(parse_temperature("invalid"), None);
        assert_eq!(parse_temperature(""), None);
        assert_eq!(parse_temperature("45"), None);
    }

    #[test]
    fn disk_status_name_values() {
        assert_eq!(disk_status_name(0), "unknown");
        assert_eq!(disk_status_name(1), "good");
        assert_eq!(disk_status_name(2), "caution");
        assert_eq!(disk_status_name(3), "bad");
        assert_eq!(disk_status_name(99), "unknown");
    }
}
