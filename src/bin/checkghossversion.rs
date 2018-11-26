extern crate dotenv;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate reqwest;
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;
extern crate structopt;
extern crate toml;

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;
use structopt::StructOpt;

include!(concat!(env!("OUT_DIR"), "/checkghossversion_token.rs"));

#[derive(StructOpt, Debug)]
#[structopt(name = "checkghossversion")]
struct Opt {
    #[structopt(name = "FILE", help = "input", parse(from_os_str))]
    filename: PathBuf,
}

#[derive(Debug, Deserialize)]
struct GithubOss {
    repo: String,
    version: String,
    prerelease: bool,
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
    tag: ResultTag,
    is_draft: bool,
    is_prerelease: bool,
    published_at: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ResultTag {
    name: String,
}

fn main() {
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

    let oss_list = match load_config(&opt.filename) {
        Some(list) => list,
        None => {
            println!("need oss list");
            exit(1);
        }
    };
    debug!("list={:?}", oss_list);
    debug!("graphql_release={}", load_graphql_release_string());

    for oss in &oss_list.oss {
        let result = retrieve_releases(&oss_list.github.host, &ghtoken, &oss);
        debug!("result={}", result);
        let mut result_list = serde_json::from_str::<Value>(&result).unwrap();
        let result_list = result_list["data"]["repository"]["releases"]["nodes"].take();
        let result_list = serde_json::from_value::<Vec<ResultRelease>>(result_list).unwrap();
        let stable_list = result_list.iter()
            .filter(|entry| !entry.is_draft &&
                ((!entry.is_prerelease) || (oss.prerelease && entry.is_prerelease)))
            .take(1)
            .collect::<Vec<_>>();
        match stable_list.first() {
            Some(release) => {
                match oss.version == release.tag.name {
                    true => println!("latest: repo={} tag={}", oss.repo, release.tag.name),
                    false => println!(
                        "new version was found: repo={} current={} latest={} url={}",
                        oss.repo, oss.version, release.tag.name, release.url),
                }
            }
            None => panic!("TODO: support tag"),
        }
    }

    info!("Bye");
}

fn retrieve_releases(host: &str, github_token: &str, oss: &GithubOss) -> String {
    let token: Vec<&str> = oss.repo.split_terminator('/').collect();
    let owner = token[0];
    let name = token[1];
    let mut client_builder = reqwest::ClientBuilder::new();

    if let Some(proxy) = get_proxy() {
        client_builder = client_builder.proxy(reqwest::Proxy::https(&proxy).unwrap());
    }

    client_builder.build().unwrap()
        .post(host)
        .bearer_auth(github_token)
        .body(json!({
            "query": load_graphql_release_string(),
            "variables": {
                "owner": owner,
                "name": name
            }
        }).to_string())
        .send().unwrap()
        .text().unwrap()
}

fn get_github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .map(|token: String| Some(token))
        .unwrap_or(None)
}

fn load_config(file_path: &Path) -> Option<GithubOssConfig> {
    let mut oss_list_file = File::open(file_path).unwrap();
    let mut oss_list_string = String::new();
    oss_list_file.read_to_string(&mut oss_list_string).unwrap();
    Some(toml::from_str(&oss_list_string).unwrap())
}

fn load_graphql_release_string() -> &'static str {
    get_checkghossversion_string()
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
