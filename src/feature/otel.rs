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

use crate::prelude::*;
use opentelemetry::KeyValue;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::logs::LoggerProvider;
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions::resource::{
    DEPLOYMENT_ENVIRONMENT, HOST_ARCH, OS_TYPE, SERVICE_INSTANCE_ID, SERVICE_NAME,
    SERVICE_NAMESPACE, SERVICE_VERSION, TELEMETRY_SDK_LANGUAGE, TELEMETRY_SDK_NAME,
    TELEMETRY_SDK_VERSION,
};
use opentelemetry_semantic_conventions::SCHEMA_URL;
use reqwest::Client;
use std::str::FromStr;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::FmtSubscriber;
use url::Url;

pub fn init_otel(
    client: Client,
    logs_endpoint: Url,
    namespace: &'static str,
    name: &'static str,
) -> Fallible<OtelGuards> {
    let logger_provider =
        create_logger_provider(create_resource(namespace, name), client, logs_endpoint)?;

    tracing_subscriber::registry()
        .with(match std::env::var("RUST_LOG") {
            Ok(var) => tracing_subscriber::filter::Targets::from_str(&var)
                .map_err(|e| eprintln!("Ignoring `RUST_LOG={:?}`: {}", var, e))
                .unwrap_or_default(),
            Err(std::env::VarError::NotPresent) => tracing_subscriber::filter::Targets::new()
                .with_default(FmtSubscriber::DEFAULT_MAX_LEVEL),
            Err(e) => {
                eprintln!("Ignoring `RUST_LOG`: {}", e);
                tracing_subscriber::filter::Targets::new()
                    .with_default(FmtSubscriber::DEFAULT_MAX_LEVEL)
            }
        })
        .with(tracing_subscriber::fmt::layer())
        .with(OpenTelemetryTracingBridge::new(&logger_provider))
        .try_init()?;

    Ok(OtelGuards {
        logger: logger_provider,
    })
}

fn create_resource(namespace: &'static str, name: &'static str) -> Resource {
    let instance_id = hostname::get()
        .expect("hostname")
        .into_string()
        .expect("invalid utf-8")
        .leak();

    Resource::from_schema_url(
        [
            KeyValue::new(SERVICE_NAMESPACE, namespace),
            KeyValue::new(SERVICE_NAME, name),
            KeyValue::new(SERVICE_INSTANCE_ID, instance_id as &'static str),
            KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
            KeyValue::new(
                DEPLOYMENT_ENVIRONMENT,
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                },
            ),
            KeyValue::new(TELEMETRY_SDK_LANGUAGE, "rust"),
            KeyValue::new(TELEMETRY_SDK_NAME, "opentelemetry"),
            KeyValue::new(TELEMETRY_SDK_VERSION, "0.24.1"),
            KeyValue::new(
                OS_TYPE,
                if cfg!(target_os = "macos") {
                    "darwin"
                } else if cfg!(target_os = "dragonfly") {
                    "dragonflybsd"
                } else if cfg!(target_os = "windows") {
                    "windows"
                } else if cfg!(target_os = "linux") {
                    "linux"
                } else if cfg!(target_os = "freebsd") {
                    "freebsd"
                } else if cfg!(target_os = "netbsd") {
                    "netbsd"
                } else if cfg!(target_os = "openbsd") {
                    "openbsd"
                } else {
                    "none"
                },
            ),
            KeyValue::new(
                HOST_ARCH,
                if cfg!(target_arch = "x86") {
                    "x86"
                } else if cfg!(target_arch = "x86_64") {
                    "amd64"
                } else if cfg!(target_arch = "arm") {
                    "arm32"
                } else if cfg!(target_arch = "aarch64") {
                    "arm64"
                } else {
                    "none"
                },
            ),
        ],
        SCHEMA_URL,
    )
}

fn create_logger_provider(
    resource: Resource,
    client: Client,
    endpoint: Url,
) -> Fallible<LoggerProvider> {
    opentelemetry_otlp::new_pipeline()
        .logging()
        .with_resource(resource)
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .http()
                .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
                .with_http_client(client)
                .with_endpoint(endpoint),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .context("failed to install logs exporter")
}

pub struct OtelGuards {
    logger: LoggerProvider,
}

impl Drop for OtelGuards {
    fn drop(&mut self) {
        self.logger.force_flush();
    }
}
