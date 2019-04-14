use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use log::{debug, info};
use regex::Regex;
use serde_derive::Deserialize;
use serde_json::{self, json, Value};
use structopt::StructOpt;

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
    tag: ResultTagName,
    is_draft: bool,
    is_prerelease: bool,
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
    dotenv::dotenv().ok();
    env_logger::init();
    info!("Hello");

    let opt: Opt = Opt::from_args();
    debug!("opt: {:?}", opt);

    let ghtoken = get_github_token().expect("need github token");

    let oss_list = load_config(&opt.filename).expect("failed to open config") as GithubOssConfig;

    debug!("list={:?}", oss_list);

    let mut client_builder = reqwest::ClientBuilder::new();

    if let Some(proxy) = get_proxy() {
        client_builder = client_builder.proxy(reqwest::Proxy::https(&proxy)?);
    }

    let body = generate_body(&oss_list.oss, false, 10)?;
    debug!("{}", body);
    let result = client_builder
        .build()?
        .post(&oss_list.github.host)
        .bearer_auth(ghtoken)
        .body(body)
        .send()?
        .text()?;
    debug!("result={}", result);

    let mut result = serde_json::from_str::<Value>(&result)?;
    let regex = Regex::new(r"[-.]")?;

    for oss in &oss_list.oss {
        let token: Vec<&str> = oss.repo.split_terminator('/').collect();
        let repo_name = regex
            .replace_all(&format!("{}_{}", token[0], token[1]), "_")
            .to_string();
        match oss.check_method {
            CheckMethod::Release => {
                let result_list = result["data"][&repo_name]["releases"]["nodes"].take();
                let result_list = serde_json::from_value::<Vec<ResultRelease>>(result_list)
                    .unwrap_or_else(|_| panic!("release not found: {}", repo_name));
                let release = result_list
                    .into_iter()
                    .filter(|entry| !entry.is_draft && (!entry.is_prerelease || oss.prerelease))
                    .take(1)
                    .collect::<Vec<_>>()
                    .pop();
                print_release(&release, &oss);
            }
            CheckMethod::Tag => {
                let result_list = result["data"][&repo_name]["refs"]["nodes"].take();
                let result_list = serde_json::from_value::<Vec<ResultTag>>(result_list)
                    .unwrap_or_else(|_| panic!("tag not found: {}", repo_name));
                let tag = result_list.into_iter().take(1).collect::<Vec<_>>().pop();
                print_tag(&tag, &oss);
            }
        }
    }

    info!("Bye");

    Ok(())
}

fn generate_body(oss_list: &[GithubOss], dry_run: bool, num: i32) -> Result<String> {
    let regex = Regex::new(r"[-.]")?;
    let mut query_body = String::new();
    for github_oss in oss_list {
        let token: Vec<&str> = github_oss.repo.split_terminator('/').collect();
        let (owner, name) = (token[0], token[1]);

        let fragment_type = match github_oss.check_method {
            CheckMethod::Release => "Rel",
            CheckMethod::Tag => "Tag",
        };
        query_body.push_str(&format!(
            r#"{}_{}: repository(owner: "{}", name: "{}") {{ ...{} }}"#,
            regex.replace_all(owner, "_"),
            regex.replace_all(name, "_"),
            owner,
            name,
            fragment_type
        ));
    }

    Ok(json!({
        "query": format!(r#"query ($dryRun: Boolean, $num: Int!) {{
{}
  rateLimit(dryRun: $dryRun) {{
    cost
    remaining
    nodeCount
  }}
}},
{}
{}"#, query_body, get_release_fragment_str(), get_tag_fragment_str()),
        "variables": {
            "dryRun": dry_run,
            "num": num
        }
    })
    .to_string())
}

fn print_release(release: &Option<ResultRelease>, oss: &GithubOss) {
    match release {
        Some(release) => {
            if oss.version == release.tag.name {
                println!("latest: repo={} tag={}", oss.repo, release.tag.name)
            } else {
                println!(
                    "new version was found: repo={} current={} latest={} url={}",
                    oss.repo, oss.version, release.tag.name, release.url
                )
            }
        }
        None => println!("release repo={} not found", oss.repo),
    }
}

fn print_tag(tag: &Option<ResultTag>, oss: &GithubOss) {
    match tag {
        Some(tag) => {
            if oss.version == tag.name {
                println!("latest: repo={} tag={}", oss.repo, tag.name)
            } else {
                println!(
                    "new version was found: repo={} current={} latest={} url={}",
                    oss.repo, oss.version, tag.name, tag.repository.url
                )
            }
        }
        None => println!("tag repo={} not found", oss.repo),
    }
}

fn load_config(file_path: &Path) -> Result<GithubOssConfig> {
    let mut oss_list_file = File::open(file_path)?;
    let mut oss_list_string = String::new();
    oss_list_file.read_to_string(&mut oss_list_string)?;
    Ok(toml::from_str(&oss_list_string)?)
}

fn get_release_fragment_str() -> &'static str {
    get_fragment_release()
}

fn get_tag_fragment_str() -> &'static str {
    get_fragment_tag()
}

fn get_github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN").ok()
}

fn get_proxy() -> Option<String> {
    std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .ok()
}
