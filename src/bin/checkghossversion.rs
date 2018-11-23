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
extern crate toml;

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

include!(concat!(env!("OUT_DIR"), "/checkghossversion_token.rs"));

#[derive(Debug, Deserialize)]
struct GithubOss {
    owner: String,
    name: String,
    version: String,
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
struct ResultViewer {
    login: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResultRateLimit {
    cost: i32,
    limit: i32,
    node_count: i32,
    remaining: i32,
    reset_at: String,
}

#[derive(Debug, Deserialize)]
struct ResultBody {
    data: ResultData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResultData {
    repository: ResultRepository,
    viewer: ResultViewer,
    rate_limit: ResultRateLimit,
}

#[derive(Debug, Deserialize)]
struct ResultRepository {
    releases: ResultReleasesNodes,
}

#[derive(Debug, Deserialize)]
struct ResultReleasesNodes {
    nodes: Vec<ResultRelease>,
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
    dotenv::dotenv().ok();
    env_logger::init();
    info!("Hello");

    let ghtoken = match get_github_token() {
        Some(token) => token,
        None => {
            println!("need github token");
            return;
        }
    };

    let oss_list = match load_config() {
        Some(list) => list,
        None => {
            println!("need oss list");
            return;
        }
    };
    debug!("list={:?}", oss_list);

    for oss in &oss_list.oss {
        let result = retrieve_releases(&oss_list.github.host, &ghtoken, &oss.owner, &oss.name);
        debug!("result={:?}", result);
        let stable_list = result.data.repository.releases.nodes.iter()
            .filter(|entry| !entry.is_draft && !entry.is_prerelease)
            .take(1)
            .collect::<Vec<&ResultRelease>>();
        match stable_list.get(0) {
            Some(release) => {
                match oss.version == release.tag.name {
                    true => println!("latest: name={}/{} tag={}",
                                     oss.owner, oss.name, release.tag.name),
                    false => println!("new version was found: name={}/{} current={} latest={} url={}",
                                      oss.owner, oss.name, oss.version, release.tag.name,
                                      release.url),
                }
            }
            None => panic!("TODO"),
        }
    }

    info!("Bye");
}

fn retrieve_releases(host: &str, github_token: &str, owner: &str, name: &str) -> ResultBody {
    let mut client_builder = reqwest::ClientBuilder::new();

    if let Some(proxy) = get_proxy() {
        client_builder = client_builder.proxy(reqwest::Proxy::https(&proxy).unwrap());
    }

    let graphql_release_string = load_graphql_release_string();

    debug!("graphql_release={}", graphql_release_string);

    client_builder.build().unwrap()
        .post(host)
        .bearer_auth(github_token)
        .body(json!({
            "query": graphql_release_string,
            "variables": {
                "owner": owner,
                "name": name
            }
        }).to_string())
        .send().unwrap()
        .json::<ResultBody>().unwrap()
}

fn get_github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .map(|token: String| Some(token))
        .unwrap_or(None)
}

fn load_config() -> Option<GithubOssConfig> {
    let file_path = match get_config_path() {
        Some(path) => path,
        None => {
            return None;
        }
    };

    let mut oss_list_file = File::open(file_path).unwrap();
    let mut oss_list_string = String::new();
    oss_list_file.read_to_string(&mut oss_list_string).unwrap();
    Some(toml::from_str(&oss_list_string).unwrap())
}

fn get_config_path() -> Option<PathBuf> {
    let mut current_path = std::env::current_dir().unwrap();
    current_path.push(get_config_name());

    if current_path.exists() {
        return Some(current_path);
    }

    let mut exe_path = get_exe_path();
    exe_path.push(get_config_name());

    match exe_path.exists() {
        true => Some(exe_path),
        false => None,
    }
}

fn get_exe_path() -> PathBuf {
    std::env::current_exe().unwrap().parent().unwrap().to_owned()
}

fn get_config_name() -> String {
    std::env::current_exe().unwrap().file_stem().unwrap().to_str().unwrap().to_string() + ".toml"
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
