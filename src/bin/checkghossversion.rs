use std::{
    fmt,
    fs::{self, File},
    io::{prelude::*, BufWriter},
    path::{Path, PathBuf},
};

use log::{debug, info, trace};
use regex::Regex;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_derive::{Deserialize, Serialize};
use serde_json::{self, json, Value};
use structopt::StructOpt;

use rust_myscript::myscript::prelude::*;

include!(concat!(env!("OUT_DIR"), "/checkghossversion_token.rs"));

#[derive(StructOpt, Debug)]
#[structopt(name = "checkghossversion")]
struct Opt {
    #[structopt(short = "v", long = "verbose", parse(from_occurrences))]
    verbose: u8,

    #[structopt(
        long = "query-per-repo",
        help = "a querying number of the tag or releases",
        default_value = "10"
    )]
    query_per_repo: i32,

    #[structopt(name = "RECIPE", help = "input", parse(from_os_str))]
    filename: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CheckMethod {
    Release,
    Tag,
}

// TODO: E0723.
const GITHUB_OSS_FIELDS: &'static [&'static str] = &[
    "repo",
    "version",
    "version_rule",
    "prerelease",
    "check_method",
];

enum GithubOssField {
    Repo,
    Version,
    VersionRule,
    Prerelease,
    CheckMethod,
}

impl GithubOssField {
    fn to_str(&self) -> &'static str {
        match self {
            GithubOssField::Repo => "repo",
            GithubOssField::Version => "version",
            GithubOssField::VersionRule => "version_rule",
            GithubOssField::Prerelease => "prerelease",
            GithubOssField::CheckMethod => "check_method",
        }
    }
}

impl<'de> Deserialize<'de> for GithubOssField {
    fn deserialize<D>(deserializer: D) -> Result<GithubOssField, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FieldVisitor;

        impl<'de> Visitor<'de> for FieldVisitor {
            type Value = GithubOssField;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("GithubOss struct fields")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match v {
                    "repo" => Ok(GithubOssField::Repo),
                    "version" => Ok(GithubOssField::Version),
                    "version_rule" => Ok(GithubOssField::VersionRule),
                    "prerelease" => Ok(GithubOssField::Prerelease),
                    "check_method" => Ok(GithubOssField::CheckMethod),
                    _ => Err(de::Error::unknown_field(v, GITHUB_OSS_FIELDS)),
                }
            }
        }

        deserializer.deserialize_identifier(FieldVisitor)
    }
}

#[derive(Debug)]
struct GithubOss {
    repo: String,
    owner: String,
    name: String,
    version: String,
    version_rule: Option<String>,
    prerelease: bool,
    check_method: CheckMethod,
}

impl GithubOss {
    fn new(
        repo: &str,
        version: &str,
        version_rule: Option<String>,
        prerelease: bool,
        check_method: CheckMethod,
    ) -> GithubOss {
        let split = repo.split('/').collect::<Vec<_>>();
        GithubOss {
            repo: repo.to_string(),
            owner: split[0].to_string(),
            name: split[1].to_string(),
            version: version.to_string(),
            version_rule,
            prerelease,
            check_method,
        }
    }
}

struct GithubOssVisitor;

impl<'de> Visitor<'de> for GithubOssVisitor {
    type Value = GithubOss;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a GithubOss struct")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        Ok(GithubOss::new(
            seq.next_element()?
                .ok_or_else(|| de::Error::invalid_length(0, &self))?,
            seq.next_element()?
                .ok_or_else(|| de::Error::invalid_length(1, &self))?,
            seq.next_element()?
                .ok_or_else(|| de::Error::invalid_length(2, &self))?,
            seq.next_element()?
                .ok_or_else(|| de::Error::invalid_length(3, &self))?,
            seq.next_element()?
                .ok_or_else(|| de::Error::invalid_length(4, &self))?,
        ))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut repo = None;
        let mut version = None;
        let mut version_rule = None;
        let mut prerelease = None;
        let mut check_method = None;
        while let Some(key) = map.next_key()? {
            match key {
                GithubOssField::Repo => {
                    if repo.is_some() {
                        return Err(de::Error::duplicate_field(GithubOssField::Repo.to_str()));
                    }
                    repo = Some(map.next_value()?);
                }
                GithubOssField::Version => {
                    if version.is_some() {
                        return Err(de::Error::duplicate_field(GithubOssField::Version.to_str()));
                    }
                    version = Some(map.next_value()?);
                }
                GithubOssField::VersionRule => {
                    if version_rule.is_some() {
                        return Err(de::Error::duplicate_field(
                            GithubOssField::VersionRule.to_str(),
                        ));
                    }
                    version_rule = Some(map.next_value()?);
                }
                GithubOssField::Prerelease => {
                    if prerelease.is_some() {
                        return Err(de::Error::duplicate_field(
                            GithubOssField::Prerelease.to_str(),
                        ));
                    }
                    prerelease = Some(map.next_value()?);
                }
                GithubOssField::CheckMethod => {
                    if check_method.is_some() {
                        return Err(de::Error::duplicate_field(
                            GithubOssField::CheckMethod.to_str(),
                        ));
                    }
                    check_method = Some(map.next_value()?);
                }
            }
        }
        Ok(GithubOss::new(
            repo.ok_or_else(|| de::Error::missing_field(GithubOssField::Repo.to_str()))?,
            version.ok_or_else(|| de::Error::missing_field(GithubOssField::Version.to_str()))?,
            version_rule,
            prerelease
                .ok_or_else(|| de::Error::missing_field(GithubOssField::Prerelease.to_str()))?,
            check_method
                .ok_or_else(|| de::Error::missing_field(GithubOssField::CheckMethod.to_str()))?,
        ))
    }
}

impl<'de> Deserialize<'de> for GithubOss {
    fn deserialize<D>(deserializer: D) -> Result<GithubOss, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_struct("GithubOss", GITHUB_OSS_FIELDS, GithubOssVisitor)
    }
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
    let opt: Opt = Opt::from_args();
    setup_log(opt.verbose)?;
    info!("Hello");
    info!("log level: {}", log::max_level());

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

    let body = generate_body(&oss_list.oss, false, opt.query_per_repo)?;
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
    let regex = Regex::new(r"[-./]")?;

    for oss in &oss_list.oss {
        let repo_name = regex.replace_all(&oss.repo, "_").to_string();
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
    let regex = Regex::new(r"[-./]")?;
    let mut query_body = String::new();
    for github_oss in oss_list {
        let fragment_type = match github_oss.check_method {
            CheckMethod::Release => "Rel",
            CheckMethod::Tag => "Tag",
        };
        query_body.push_str(&format!(
            r#"{}: repository(owner: "{}", name: "{}") {{ ...{} }}"#,
            regex.replace_all(&github_oss.repo, "_"),
            github_oss.owner,
            github_oss.name,
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
                info!("latest: repo={} tag={}", oss.repo, release.tag.name)
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
                info!("latest: repo={} tag={}", oss.repo, tag.name)
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

fn setup_log(level: u8) -> Fallible<()> {
    use log::LevelFilter::*;

    let mut builder = env_logger::Builder::from_default_env();
    let builder = match level {
        0 => &mut builder,
        1 => builder.filter_level(Info),
        2 => builder.filter_level(Debug),
        _ => builder.filter_level(Trace),
    };
    Ok(builder.try_init()?)
}
