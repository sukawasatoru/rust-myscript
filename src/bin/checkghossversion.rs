use std::{
    fs::{self, File},
    io::{prelude::*, BufWriter},
    path::{Path, PathBuf},
};

use log::{debug, info, trace};
use regex::Regex;
use serde_derive::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use structopt::StructOpt;

use rust_myscript::myscript::prelude::*;

include!(concat!(env!("OUT_DIR"), "/checkghossversion_token.rs"));

#[derive(StructOpt, Debug)]
#[structopt(name = "checkghossversion")]
struct Opt {
    #[structopt(name = "RECIPE", help = "input", parse(from_os_str))]
    filename: Option<PathBuf>,
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
    version_rule: Option<String>,
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

#[derive(Debug, Deserialize, Serialize)]
struct Config {
    default_recipe: Option<PathBuf>,
    github: GitHubConfig,
}

impl Config {
    fn new() -> Self {
        Default::default()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_recipe: Default::default(),
            github: Default::default(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct GitHubConfig {
    oauth_token: String,
}

impl Default for GitHubConfig {
    fn default() -> Self {
        Self {
            oauth_token: Default::default(),
        }
    }
}

fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    env_logger::init();
    info!("Hello");
    info!("log level: {}", log::max_level());

    let opt: Opt = Opt::from_args();
    debug!("opt: {:?}", opt);

    let project_dirs =
        directories::ProjectDirs::from("jp", "tinyport", "checkghossversion").ok_or_err()?;
    let config_path = project_dirs.config_dir().join("config.toml");
    let mut toml_loader = TomlLoader::new();
    let config = prepare_config(&mut toml_loader, &config_path)?;
    let ghtoken = get_github_token(&config).expect("need github token");

    let recipe_path = &opt
        .filename
        .or_else(|| match config.default_recipe {
            ref default_recipe @ Some(_) => {
                info!("use recipe path via config");
                default_recipe.clone()
            }
            None => None,
        })
        .expect("the recipe is required. specify via command line or config file");
    let oss_list = toml_loader
        .load::<GithubOssConfig>(&recipe_path)
        .expect("failed to open a recipe");

    trace!("list={:?}", oss_list);

    let mut client_builder = reqwest::ClientBuilder::new();

    if let Some(proxy) = get_proxy() {
        client_builder = client_builder.proxy(reqwest::Proxy::https(&proxy)?);
    }

    let body = generate_body(&oss_list.oss, false, 10)?;
    trace!("{}", body);
    let result = client_builder
        .build()?
        .post(&oss_list.github.host)
        .bearer_auth(ghtoken)
        .body(body)
        .send()?
        .text()?;
    trace!("result={}", result);

    let mut result = serde_json::from_str::<Value>(&result)?;
    let regex = Regex::new(r"[-.]")?;

    for oss in &oss_list.oss {
        let token: Vec<&str> = oss.repo.split_terminator('/').collect();
        let repo_name = regex
            .replace_all(&format!("{}_{}", token[0], token[1]), "_")
            .to_string();
        let version_reg = match oss.version_rule {
            Some(ref rule) => {
                debug!("{} use version_rules: {}", oss.repo, rule);
                Some(regex::Regex::new(rule)?)
            }
            None => None,
        };
        match oss.check_method {
            CheckMethod::Release => {
                let result_list = result["data"][&repo_name]["releases"]["nodes"].take();
                let result_list = serde_json::from_value::<Vec<ResultRelease>>(result_list)
                    .unwrap_or_else(|_| panic!("release not found: {}", repo_name));
                let release = result_list
                    .into_iter()
                    .filter(|entry| !entry.is_draft && (!entry.is_prerelease || oss.prerelease))
                    .filter(|entry| match version_reg {
                        Some(ref reg) => reg.is_match(&entry.tag.name),
                        None => true,
                    })
                    .take(1)
                    .collect::<Vec<_>>()
                    .pop();
                print_release(&release, &oss);
            }
            CheckMethod::Tag => {
                let result_list = result["data"][&repo_name]["refs"]["nodes"].take();
                let result_list = serde_json::from_value::<Vec<ResultTag>>(result_list)
                    .unwrap_or_else(|_| panic!("tag not found: {}", repo_name));
                let tag = result_list
                    .into_iter()
                    .filter(|entry| match version_reg {
                        Some(ref reg) => reg.is_match(&entry.name),
                        None => true,
                    })
                    .take(1)
                    .collect::<Vec<_>>()
                    .pop();
                print_tag(&tag, &oss);
            }
        }
    }

    info!("Bye");

    Ok(())
}

fn generate_body(oss_list: &[GithubOss], dry_run: bool, num: i32) -> Fallible<String> {
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

fn prepare_config(loader: &mut TomlLoader, path: &Path) -> Fallible<Config> {
    if path.exists() {
        return loader.load(path);
    }

    info!("create new config file");
    let dir = path.parent().ok_or_err()? as &Path;
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }
    let config = Config::new();
    let mut buffer = BufWriter::new(File::create(path)?);
    buffer.write_all(&toml::to_vec(&config)?)?;
    eprintln!("Config file created successfully: {:?}", path);
    Ok(config)
}

fn get_release_fragment_str() -> &'static str {
    get_fragment_release()
}

fn get_tag_fragment_str() -> &'static str {
    get_fragment_tag()
}

fn get_github_token(config: &Config) -> Option<String> {
    std::env::var("GITHUB_TOKEN").ok().or_else(|| {
        if config.github.oauth_token.is_empty() {
            None
        } else {
            Some(config.github.oauth_token.to_string())
        }
    })
}

fn get_proxy() -> Option<String> {
    std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .ok()
}
