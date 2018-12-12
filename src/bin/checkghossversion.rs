use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use dotenv;
use env_logger;
use log::{debug, info};
use reqwest;
use serde_derive::Deserialize;
use serde_json::{self, json, Value};
use structopt::StructOpt;
use toml;

use rust_myscript::myscript::prelude::*;

include!(concat!(env!("OUT_DIR"), "/checkghossversion_token.rs"));

#[derive(StructOpt, Debug)]
#[structopt(name = "checkghossversion")]
struct Opt {
    #[structopt(name = "FILE", help = "input", parse(from_os_str))]
    filename: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CheckMethod {
    Release,
    Tag,
}

#[derive(Debug, Deserialize)]
struct GithubOss {
    repo: String,
    version: String,
    prerelease: bool,
    check_method: CheckMethod,
}

#[derive(Debug, Deserialize)]
struct GithubConfig {
    host: String,
}

#[derive(Debug, Deserialize)]
struct GithubOssConfig {
    github: GithubConfig,
    oss: Vec<GithubOss>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResultRelease {
    name: String,
    tag: ResultTagName,
    is_draft: bool,
    is_prerelease: bool,
    published_at: String,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResultTag {
    name: String,
    repository: ResultRepository,
}

#[derive(Debug, Deserialize)]
struct ResultTagName {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ResultRepository {
    url: String,
}

fn main() -> Result<()> {
    use std::process::exit;

    dotenv::dotenv().ok();
    env_logger::init();
    info!("Hello");

    let opt: Opt = Opt::from_args();
    debug!("opt: {:?}", opt);

    let ghtoken = match get_github_token() {
        Some(token) => token,
        None => {
            println!("need github token");
            exit(1);
        }
    };

    let oss_list: GithubOssConfig = match load_config(&opt.filename) {
        Ok(list) => list,
        Err(e) => {
            println!("failed to open config: {:?}", e);
            exit(1);
        }
    };
    debug!("list={:?}", oss_list);

    for oss in &oss_list.oss {
        match oss.check_method {
            CheckMethod::Release => {
                let result: String = retrieve_releases(&oss_list.github.host, &ghtoken, &oss)?;
                debug!("result={}", result);
                let release = filter_latest_release(&result, &oss)?;
                print_release(&release, &oss);
            }
            CheckMethod::Tag => {
                let result = retrieve_tag(&oss_list.github.host, &ghtoken, &oss)?;
                debug!("result={}", result);
                let tag = filter_latest_tag(&result)?;
                print_tag(&tag, &oss);
            }
        }
    }

    info!("Bye");

    Ok(())
}

fn print_release(release: &Option<ResultRelease>, oss: &GithubOss) {
    match release {
        Some(release) => {
            match oss.version == release.tag.name {
                true => println!("latest: repo={} tag={}", oss.repo, release.tag.name),
                false => println!(
                    "new version was found: repo={} current={} latest={} url={}",
                    oss.repo, oss.version, release.tag.name, release.url),
            }
        }
        None => println!("release repo={} not found", oss.repo),
    }
}

fn print_tag(tag: &Option<ResultTag>, oss: &GithubOss) {
    match tag {
        Some(tag) => {
            match oss.version == tag.name {
                true => println!("latest: repo={} tag={}", oss.repo, tag.name),
                false => println!(
                    "new version was found: repo={} current={} latest={} url={}",
                    oss.repo, oss.version, tag.name, tag.repository.url),
            }
        }
        None => println!("tag repo={} not found", oss.repo),
    }
}

fn filter_latest_release(releases_str: &str, oss: &GithubOss) -> Result<Option<ResultRelease>> {
    let mut result_list = serde_json::from_str::<Value>(releases_str)?;
    let result_list = result_list["data"]["repository"]["releases"]["nodes"].take();
    let result_list = serde_json::from_value::<Vec<ResultRelease>>(result_list)?;
    let ret = result_list.into_iter()
        .filter(|entry| !entry.is_draft &&
            ((!entry.is_prerelease) || (oss.prerelease && entry.is_prerelease)))
        .take(1)
        .collect::<Vec<_>>()
        .pop();
    Ok(ret)
}

fn filter_latest_tag(tags_str: &str) -> Result<Option<ResultTag>> {
    let mut result_list = serde_json::from_str::<Value>(tags_str)?;
    let result_list = result_list["data"]["repository"]["refs"]["nodes"].take();
    let result_list = serde_json::from_value::<Vec<ResultTag>>(result_list)?;
    let ret = result_list.into_iter()
        .take(1)
        .collect::<Vec<_>>()
        .pop();
    Ok(ret)
}

fn retrieve_releases(host: &str, github_token: &str, oss: &GithubOss) -> Result<String> {
    let token: Vec<&str> = oss.repo.split_terminator('/').collect();
    let owner = token[0];
    let name = token[1];
    let mut client_builder = reqwest::ClientBuilder::new();

    if let Some(proxy) = get_proxy() {
        client_builder = client_builder.proxy(reqwest::Proxy::https(&proxy)?);
    }

    let ret = client_builder.build()?
        .post(host)
        .bearer_auth(github_token)
        .body(json!({
            "query": load_graphql_release_string(),
            "variables": {
                "owner": owner,
                "name": name
            }
        }).to_string())
        .send()?
        .text()?;
    Ok(ret)
}

fn retrieve_tag(host: &str, github_token: &str, oss: &GithubOss) -> Result<String> {
    let token: Vec<&str> = oss.repo.split_terminator('/').collect();
    let owner = token[0];
    let name = token[1];
    let mut client_builder = reqwest::ClientBuilder::new();

    if let Some(proxy) = get_proxy() {
        client_builder = client_builder.proxy(reqwest::Proxy::https(&proxy)?);
    }

    let ret = client_builder.build()?
        .post(host)
        .bearer_auth(github_token)
        .body(json!({
            "query": load_graphql_tag_string(),
            "variables": {
                "owner": owner,
                "name": name
            }
        }).to_string())
        .send()?
        .text()?;
    Ok(ret)
}

fn get_github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .map(|token: String| Some(token))
        .unwrap_or(None)
}

fn load_config(file_path: &Path) -> Result<GithubOssConfig> {
    let mut oss_list_file = File::open(file_path)?;
    let mut oss_list_string = String::new();
    oss_list_file.read_to_string(&mut oss_list_string)?;
    Ok(toml::from_str(&oss_list_string)?)
}

fn load_graphql_release_string() -> &'static str {
    get_graphql_release()
}

fn load_graphql_tag_string() -> &'static str {
    get_graphql_tag()
}

fn get_proxy() -> Option<String> {
    use std::env;

    if let Ok(proxy) = env::var("HTTPS_PROXY") {
        return Some(proxy);
    }

    env::var("https_proxy")
        .map(|proxy: String| Some(proxy))
        .unwrap_or(None)
}
