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
use filededuplication::feature::remove_file::remove_file;
use filededuplication::feature::remove_file::test_helpers::Target::{Directory, File};
use filededuplication::feature::remove_file::test_helpers::{create_target, init_tracing};
use std::env::set_current_dir;
use std::path::Path;

#[tokio::test]
async fn remove_file_if_master_and_shrink_are_relative() {
    let temp_dir = create_target(&[File("a/file"), File("b/file"), Directory("c")]);

    set_current_dir(temp_dir.path().join("c")).unwrap();

    let (_guard, log_writer) = init_tracing();

    remove_file(
        false,
        None,
        Path::new("..").join("a"),
        Path::new("..").join("b"),
    )
    .await
    .unwrap();

    let actual_messages = log_writer.remove();
    let mut actual_messages = actual_messages.trim_end().split('\n');
    assert_eq!(
        format!(
            "remove path={}",
            Path::new("..").join("b").join("file").display()
        ),
        actual_messages.next().unwrap(),
    );
    assert_eq!(
        "complete all=1 remove=1 skip=0 error=0",
        actual_messages.next().unwrap(),
    );
    assert!(actual_messages.next().is_none());

    assert!(temp_dir.path().join("a/file").exists());
    assert!(!temp_dir.path().join("b/file").exists());
}
