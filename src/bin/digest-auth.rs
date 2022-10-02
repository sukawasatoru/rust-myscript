use clap::Parser;
use digest::{Digest, FixedOutputReset};
use rust_myscript::prelude::*;
use std::str::FromStr;
use tracing::{debug, info, warn};
use warp::http::header;
use warp::http::StatusCode;
use warp::Filter;

#[derive(Debug, Parser)]
struct Opt {
    #[arg(long)]
    password: String,

    #[arg(short, long)]
    realm: String,

    #[arg(long)]
    port: u16,
}

/// - https://tools.ietf.org/html/rfc7616#page-28
/// - https://tools.ietf.org/html/rfc3230#section-4.1.1
/// - https://tools.ietf.org/html/rfc5843
/// - https://www.iana.org/assignments/http-dig-alg/http-dig-alg.xhtml
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum Algorithm {
    Md5,
    Sha,
    Sha256,
    Sha512,
}

impl Algorithm {
    fn rfc_name(&self) -> &'static str {
        match self {
            Algorithm::Md5 => "MD5",
            Algorithm::Sha => "SHA",
            Algorithm::Sha256 => "SHA-256",
            Algorithm::Sha512 => "SHA-512",
        }
    }

    fn digest_auth(
        &self,
        http_method: &str,
        username: &str,
        password: &str,
        realm: &str,
        nonce: &str,
        uri: &str,
        cnonce: &str,
        nc: &str,
    ) -> String {
        match self {
            Algorithm::Md5 => {
                let mut digest = md5::Md5::new();
                digest_auth(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                )
            }
            Algorithm::Sha => {
                let mut digest = sha1::Sha1::new();
                digest_auth(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                )
            }
            Algorithm::Sha256 => {
                let mut digest = sha2::Sha256::new();
                digest_auth(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                )
            }
            Algorithm::Sha512 => {
                let mut digest = sha2::Sha512::new();
                digest_auth(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                )
            }
        }
    }

    fn digest_sess_auth(
        &self,
        http_method: &str,
        username: &str,
        password: &str,
        realm: &str,
        nonce: &str,
        uri: &str,
        cnonce: &str,
        nc: &str,
    ) -> String {
        match self {
            Algorithm::Md5 => {
                let mut digest = md5::Md5::new();
                digest_sess_auth(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                )
            }
            Algorithm::Sha => {
                let mut digest = sha1::Sha1::new();
                digest_sess_auth(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                )
            }
            Algorithm::Sha256 => {
                let mut digest = sha2::Sha256::new();
                digest_sess_auth(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                )
            }
            Algorithm::Sha512 => {
                let mut digest = sha2::Sha512::new();
                digest_sess_auth(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                )
            }
        }
    }

    fn digest_auth_int(
        &self,
        http_method: &str,
        username: &str,
        password: &str,
        realm: &str,
        nonce: &str,
        uri: &str,
        cnonce: &str,
        nc: &str,
        body: &[u8],
    ) -> String {
        match self {
            Algorithm::Md5 => {
                let mut digest = md5::Md5::new();
                digest_auth_int(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                    body,
                )
            }
            Algorithm::Sha => {
                let mut digest = sha1::Sha1::new();
                digest_auth_int(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                    body,
                )
            }
            Algorithm::Sha256 => {
                let mut digest = sha2::Sha256::new();
                digest_auth_int(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                    body,
                )
            }
            Algorithm::Sha512 => {
                let mut digest = sha2::Sha512::new();
                digest_auth_int(
                    &mut digest,
                    http_method,
                    username,
                    password,
                    realm,
                    nonce,
                    uri,
                    cnonce,
                    nc,
                    body,
                )
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Qop {
    Auth,
    AuthInt,
}

impl std::str::FromStr for Qop {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "auth" => Ok(Qop::Auth),
            "auth-int" => Ok(Qop::AuthInt),
            _ => Err(anyhow::anyhow!("unexpected qop: {}", s)),
        }
    }
}

#[derive(Debug, serde::Deserialize)]
struct DigestQuery {
    #[serde(rename = "valid")]
    ignore_cert_error: Option<String>,

    algorithm: Option<Algorithm>,
}

struct HexFormat<'a>(&'a [u8]);

impl<'a> std::fmt::Display for HexFormat<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }

        write!(f, "{:02x?}", self.0[0])?;

        for entry in &self.0[1..self.0.len()] {
            write!(f, "{:02x?}", entry)?;
        }

        Ok(())
    }
}

struct Context {
    password: String,
    realm: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("Hello");

    let opt: Opt = Opt::parse();

    let filter_auth_header = warp::header::optional("Authorization");
    let ctx = std::sync::Arc::new(Context {
        password: opt.password,
        realm: opt.realm,
    });

    let digest_2069_ctx = ctx.clone();
    let digest_2069 = warp::get()
        .and(warp::path!("digest_2069"))
        .and(filter_auth_header)
        .and(warp::query::<DigestQuery>())
        .map(
            move |header_authorization: Option<String>, digest_query: DigestQuery| {
                let ctx = digest_2069_ctx.clone();
                info!(?header_authorization, ?digest_query, "rfc2069");

                let www_auth = warp::http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(
                        header::WWW_AUTHENTICATE,
                        format!(
                            r#"Digest realm="{}", nonce="{}", algorithm=MD5"#,
                            ctx.realm,
                            uuid::Uuid::new_v4()
                        ),
                    )
                    .body("ng".to_owned());

                let header_authorization = match header_authorization {
                    Some(data) => match DigestHeaderParameters::from_str(&data) {
                        Ok(d) => d,
                        Err(e) => {
                            info!(?e);
                            return www_auth;
                        }
                    },
                    None => {
                        info!("1st challenge");
                        return www_auth;
                    }
                };

                if digest_query.ignore_cert_error.is_some() {
                    info!("accept");
                    return warp::http::Response::builder().body("ok\n".to_owned());
                }

                let mut digest = md5::Md5::new();
                let calc_hash = digest_2069(
                    &mut digest,
                    "GET",
                    &header_authorization.username,
                    &ctx.password,
                    &header_authorization.realm,
                    &header_authorization.nonce,
                    &header_authorization.uri,
                );

                if calc_hash == header_authorization.response {
                    info!("accept");
                    warp::http::Response::builder().body("ok\n".to_owned())
                } else {
                    info!("reject hash");
                    www_auth
                }
            },
        );

    let md5_auth_ctx = ctx.clone();
    let md5_auth = warp::get()
        .and(warp::path!("md5_auth"))
        .and(filter_auth_header)
        .and(warp::query::<DigestQuery>())
        .map(
            move |header_authorization: Option<String>, digest_query: DigestQuery| {
                let ctx = md5_auth_ctx.clone();
                info!(?header_authorization, ?digest_query, "md5_auth");

                let algorithm = digest_query.algorithm.unwrap_or(Algorithm::Md5);
                let www_auth = warp::http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(
                        header::WWW_AUTHENTICATE,
                        format!(
                            r#"Digest realm="{}", nonce="{}", algorithm={}, qop="auth""#,
                            ctx.realm,
                            uuid::Uuid::new_v4(),
                            algorithm.rfc_name()
                        ),
                    )
                    .body("ng".to_owned());

                let header_authorization = match header_authorization {
                    Some(data) => match DigestHeaderParameters::from_str(&data) {
                        Ok(d) => d,
                        Err(e) => {
                            info!(?e);
                            return www_auth;
                        }
                    },
                    None => {
                        info!("1st challenge");
                        return www_auth;
                    }
                };

                if header_authorization.qop.is_none() {
                    info!("reject rfc 2069");
                    return www_auth;
                }

                if digest_query.ignore_cert_error.is_some() {
                    info!("accept");
                    return warp::http::Response::builder().body("ok\n".to_owned());
                }

                let cnonce = match &header_authorization.cnonce {
                    Some(d) => d.as_str(),
                    None => {
                        warn!("missing cnonce");
                        return www_auth;
                    }
                };

                let nc = match &header_authorization.nc {
                    Some(d) => d.as_str(),
                    None => {
                        warn!("missing nc");
                        return www_auth;
                    }
                };

                let calc_hash = algorithm.digest_auth(
                    "GET",
                    &header_authorization.username,
                    &ctx.password,
                    &header_authorization.realm,
                    &header_authorization.nonce,
                    &header_authorization.uri,
                    cnonce,
                    nc,
                );

                if calc_hash == header_authorization.response {
                    info!("accept");
                    warp::http::Response::builder()
                        .header(
                            "Authentication-Info",
                            format!(
                                r#"rspauth="{}", cnonce="{}", nc={}, qop={}"#,
                                algorithm.digest_auth(
                                    "",
                                    &header_authorization.username,
                                    &ctx.password,
                                    &header_authorization.realm,
                                    &header_authorization.nonce,
                                    &header_authorization.uri,
                                    cnonce,
                                    nc,
                                ),
                                cnonce,
                                nc,
                                "auth"
                            ),
                        )
                        .body("ok\n".to_owned())
                } else {
                    info!("reject hash");
                    www_auth
                }
            },
        );

    let md5_sess_auth_ctx = ctx.clone();
    let md5_sess_auth = warp::get()
        .and(warp::path!("md5_sess_auth"))
        .and(filter_auth_header)
        .and(warp::query::<DigestQuery>())
        .map(
            move |header_authorization: Option<String>, digest_query: DigestQuery| {
                let ctx = md5_sess_auth_ctx.clone();
                info!(?header_authorization, ?digest_query, "md5_sess_auth");

                let algorithm = digest_query.algorithm.unwrap_or(Algorithm::Md5);
                let www_auth = warp::http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(
                        header::WWW_AUTHENTICATE,
                        format!(
                            r#"Digest realm="{}", nonce="{}", algorithm={}-sess, qop="auth""#,
                            ctx.realm,
                            uuid::Uuid::new_v4(),
                            algorithm.rfc_name()
                        ),
                    )
                    .body("ng".to_owned());

                let header_authorization = match header_authorization {
                    Some(data) => match DigestHeaderParameters::from_str(&data) {
                        Ok(d) => d,
                        Err(e) => {
                            info!(?e);
                            return www_auth;
                        }
                    },
                    None => {
                        info!("1st challenge");
                        return www_auth;
                    }
                };

                if digest_query.ignore_cert_error.is_some() {
                    info!("accept");
                    return warp::http::Response::builder().body("ok\n".to_owned());
                }

                if header_authorization.qop.is_none() {
                    info!("reject rfc 2069");
                    return www_auth;
                }

                let cnonce = match &header_authorization.cnonce {
                    Some(d) => d.as_str(),
                    None => {
                        warn!("missing cnonce");
                        return www_auth;
                    }
                };

                let nc = match &header_authorization.nc {
                    Some(d) => d.as_str(),
                    None => {
                        warn!("missing nc");
                        return www_auth;
                    }
                };

                let calc_hash = algorithm.digest_sess_auth(
                    "GET",
                    &header_authorization.username,
                    &ctx.password,
                    &header_authorization.realm,
                    &header_authorization.nonce,
                    &header_authorization.uri,
                    cnonce,
                    nc,
                );

                if calc_hash == header_authorization.response {
                    info!("accept");
                    warp::http::Response::builder()
                        .header(
                            "Authentication-Info",
                            format!(
                                r#"rspauth="{}", cnonce="{}", nc={}, qop={}"#,
                                algorithm.digest_sess_auth(
                                    "",
                                    &header_authorization.username,
                                    &ctx.password,
                                    &header_authorization.realm,
                                    &header_authorization.nonce,
                                    &header_authorization.uri,
                                    cnonce,
                                    nc,
                                ),
                                cnonce,
                                nc,
                                "auth"
                            ),
                        )
                        .body("ok\n".to_owned())
                } else {
                    info!("reject hash");
                    www_auth
                }
            },
        );

    let md5_auth_int_jump = warp::get().and(warp::path!("md5_auth_int_jump")).map(|| {
        warp::http::Response::new(
            r#"
<!DOCTYPE html>
<head>
<title>bootstrap</title>
</head>
<body>
<form action="/md5_auth_int" method="post">
  <input type="text" name="input-value" />
  <br>
  <input type="submit">
</form>
</body>
"#,
        )
    });

    let md5_auth_int_ctx = ctx.clone();
    let md5_auth_int = warp::get()
        .and(warp::path!("md5_auth_int"))
        .and(filter_auth_header)
        .and(warp::query::<DigestQuery>())
        .and(warp::body::bytes())
        .map(
            move |header_authorization: Option<String>,
                  digest_query: DigestQuery,
                  body: bytes::Bytes| {
                let ctx = md5_auth_int_ctx.clone();
                info!(
                    ?header_authorization,
                    ?digest_query,
                    body = ?String::from_utf8_lossy(&body),
                    "md5_auth_int (GET)"
                );

                let algorithm = digest_query.algorithm.unwrap_or(Algorithm::Md5);
                let www_auth = warp::http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(
                        header::WWW_AUTHENTICATE,
                        format!(
                            r#"Digest realm="{}", nonce="{}", algorithm={}, qop="auth-int""#,
                            ctx.realm,
                            uuid::Uuid::new_v4(),
                            algorithm.rfc_name()
                        ),
                    )
                    .body("".to_owned());

                let header_authorization = match header_authorization {
                    Some(data) => match DigestHeaderParameters::from_str(&data) {
                        Ok(d) => d,
                        Err(e) => {
                            info!(?e);
                            return www_auth;
                        }
                    },
                    None => {
                        info!("1st challenge");
                        return www_auth;
                    }
                };

                if digest_query.ignore_cert_error.is_some() {
                    info!("accept");
                    return warp::http::Response::builder().body("ok\n".to_owned());
                }

                if header_authorization.qop.is_none() {
                    info!("reject rfc 2069");
                    return www_auth;
                }

                let cnonce = match &header_authorization.cnonce {
                    Some(d) => d.as_str(),
                    None => {
                        warn!("missing cnonce");
                        return www_auth;
                    }
                };

                let nc = match &header_authorization.nc {
                    Some(d) => d.as_str(),
                    None => {
                        warn!("missing nc");
                        return www_auth;
                    }
                };

                let calc_hash = algorithm.digest_auth_int(
                    "GET",
                    &header_authorization.username,
                    &ctx.password,
                    &header_authorization.realm,
                    &header_authorization.nonce,
                    &header_authorization.uri,
                    cnonce,
                    nc,
                    &body,
                );

                if calc_hash == header_authorization.response {
                    info!("accept");
                    warp::http::Response::builder()
                        .header(
                            "Authentication-Info",
                            format!(
                                r#"rspauth="{}", cnonce="{}", nc={}, qop={}"#,
                                algorithm.digest_auth_int(
                                    "",
                                    &header_authorization.username,
                                    &ctx.password,
                                    &header_authorization.realm,
                                    &header_authorization.nonce,
                                    &header_authorization.uri,
                                    cnonce,
                                    nc,
                                    &body
                                ),
                                cnonce,
                                nc,
                                "auth"
                            ),
                        )
                        .body("ok\n".to_owned())
                } else {
                    info!("reject hash");
                    www_auth
                }
            },
        );

    let md5_auth_int_post_ctx = ctx.clone();
    let md5_auth_int_post = warp::post()
        .and(warp::path!("md5_auth_int"))
        .and(filter_auth_header)
        .and(warp::query::<DigestQuery>())
        .and(warp::body::bytes())
        .map(
            move |header_authorization: Option<String>,
                  digest_query: DigestQuery,
                  body: bytes::Bytes| {
                let ctx = md5_auth_int_post_ctx.clone();
                info!(
                    ?header_authorization,
                    ?digest_query,
                    body = ?String::from_utf8_lossy(&body),
                    "md5_auth_int (POST)",
                );

                let algorithm = digest_query.algorithm.unwrap_or(Algorithm::Md5);
                let www_auth = warp::http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(
                        header::WWW_AUTHENTICATE,
                        format!(
                            r#"Digest realm="{}", nonce="{}", algorithm={}, qop="auth-int""#,
                            ctx.realm,
                            uuid::Uuid::new_v4(),
                            algorithm.rfc_name()
                        ),
                    )
                    .body("".to_owned());

                let header_authorization = match header_authorization {
                    Some(data) => match DigestHeaderParameters::from_str(&data) {
                        Ok(d) => d,
                        Err(e) => {
                            info!(?e);
                            return www_auth;
                        }
                    },
                    None => {
                        info!("1st challenge");
                        return www_auth;
                    }
                };

                if digest_query.ignore_cert_error.is_some() {
                    info!("accept");
                    return warp::http::Response::builder().body("ok\n".to_owned());
                }

                if header_authorization.qop.is_none() {
                    info!("reject rfc 2069");
                    return www_auth;
                }

                let cnonce = match &header_authorization.cnonce {
                    Some(d) => d.as_str(),
                    None => {
                        warn!("missing cnonce");
                        return www_auth;
                    }
                };

                let nc = match &header_authorization.nc {
                    Some(d) => d.as_str(),
                    None => {
                        warn!("missing nc");
                        return www_auth;
                    }
                };

                let calc_hash = algorithm.digest_auth_int(
                    "POST",
                    &header_authorization.username,
                    &ctx.password,
                    &header_authorization.realm,
                    &header_authorization.nonce,
                    &header_authorization.uri,
                    cnonce,
                    nc,
                    &body,
                );

                if calc_hash == header_authorization.response {
                    info!("accept");
                    warp::http::Response::builder()
                        .header(
                            "Authentication-Info",
                            format!(
                                r#"rspauth="{}", cnonce="{}", nc={}, qop={}"#,
                                algorithm.digest_auth_int(
                                    "",
                                    &header_authorization.username,
                                    &ctx.password,
                                    &header_authorization.realm,
                                    &header_authorization.nonce,
                                    &header_authorization.uri,
                                    cnonce,
                                    nc,
                                    &body
                                ),
                                cnonce,
                                nc,
                                "auth"
                            ),
                        )
                        .body("ok\n".to_owned())
                } else {
                    info!("reject hash");
                    www_auth
                }
            },
        );

    warp::serve(
        digest_2069
            .or(md5_auth)
            .or(md5_auth_int_jump)
            .or(md5_sess_auth)
            .or(md5_auth_int)
            .or(md5_auth_int_post),
    )
    .run(([0, 0, 0, 0], opt.port))
    .await;

    info!("Bye");
    Ok(())
}

fn update_parameters_to_digest<D, P, E>(digest: &mut D, params: P)
where
    D: Digest,
    P: AsRef<[E]>,
    E: AsRef<[u8]>,
{
    let params = params.as_ref();
    if params.is_empty() {
        return;
    }

    digest.update(params[0].as_ref());

    for param in &params[1..params.len()] {
        digest.update(":");
        digest.update(param);
    }
}

fn digest_auth<D: Digest + FixedOutputReset>(
    digest: &mut D,
    http_method: &str,
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    uri: &str,
    cnonce: &str,
    nc: &str,
) -> String {
    Digest::reset(digest);

    update_parameters_to_digest(digest, [username, realm, password]);
    let a1 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [http_method, uri]);
    let a2 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [&a1, nonce, nc, cnonce, "auth", &a2]);
    HexFormat(digest.finalize_reset().as_slice()).to_string()
}

fn digest_sess_auth<D: Digest + FixedOutputReset>(
    digest: &mut D,
    http_method: &str,
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    uri: &str,
    cnonce: &str,
    nc: &str,
) -> String {
    Digest::reset(digest);

    update_parameters_to_digest(digest, [username, realm, password]);
    let a1_prefix = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [&a1_prefix, nonce, cnonce]);
    let a1 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [http_method, uri]);
    let a2 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [&a1, nonce, nc, cnonce, "auth", &a2]);
    HexFormat(digest.finalize_reset().as_slice()).to_string()
}

fn digest_2069<D: Digest + FixedOutputReset>(
    digest: &mut D,
    http_method: &str,
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    uri: &str,
) -> String {
    Digest::reset(digest);

    update_parameters_to_digest(digest, [username, realm, password]);
    let a1 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [http_method, uri]);
    let a2 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [&a1, nonce, &a2]);
    HexFormat(digest.finalize_reset().as_slice()).to_string()
}

fn digest_auth_int<D: Digest + FixedOutputReset>(
    digest: &mut D,
    http_method: &str,
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    uri: &str,
    cnonce: &str,
    nc: &str,
    body: &[u8],
) -> String {
    Digest::reset(digest);

    update_parameters_to_digest(digest, [username, realm, password]);
    let a1 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    if !body.is_empty() {
        Digest::update(digest, body);
    }
    let body_hash = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [http_method, uri, &body_hash]);
    let a2 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [&a1, nonce, nc, cnonce, "auth-int", &a2]);
    HexFormat(digest.finalize_reset().as_slice()).to_string()
}

#[derive(Debug, Default)]
struct DigestHeaderParameters {
    username: String,
    realm: String,
    nonce: String,
    uri: String,
    cnonce: Option<String>,
    nc: Option<String>,
    qop: Option<Qop>,
    response: String,
    #[allow(dead_code)]
    algorithm: Option<String>,
}

fn get_non_unq_value(segment: &str) -> Option<&str> {
    // orig: abc123=value
    match segment.find('=') {
        Some(index) => Some(&segment[index + 1..segment.len()]),
        None => {
            warn!(%segment, "unexpected format");
            None
        }
    }
}

fn get_unq_value(segment: &str) -> Option<&str> {
    // orig: abc123="value"
    match segment.find(r#"=""#) {
        Some(index) => Some(&segment[index + 2..segment.len() - 1]),
        None => {
            warn!(%segment, "unexpected format");
            None
        }
    }
}

impl std::str::FromStr for DigestHeaderParameters {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("Digest ") {
            anyhow::bail!("the value should start with \"Digest\"");
        }

        let s = &s["Digest ".len()..];
        let segments = s.split(", ");

        let mut username = None;
        let mut realm = None;
        let mut nonce = None;
        let mut uri = None;
        let mut cnonce = None;
        let mut nc = None;
        let mut qop = None;
        let mut response = None;
        let mut algorithm = None;

        for entry in segments {
            if username.is_none() && entry.starts_with(r#"username=""#) {
                username = get_unq_value(entry);
            } else if realm.is_none() && entry.starts_with(r#"realm=""#) {
                realm = get_unq_value(entry);
            } else if nonce.is_none() && entry.starts_with(r#"nonce=""#) {
                nonce = get_unq_value(entry);
            } else if uri.is_none() && entry.starts_with(r#"uri=""#) {
                uri = get_unq_value(entry);
            } else if cnonce.is_none() && entry.starts_with(r#"cnonce=""#) {
                cnonce = get_unq_value(entry);
            } else if nc.is_none() && entry.starts_with("nc=") {
                nc = get_non_unq_value(entry);
            } else if qop.is_none() && entry.starts_with(r#"qop=""#) {
                // get_unq_value for safari.
                qop = match get_unq_value(entry) {
                    Some(d) => Some(Qop::from_str(d)?),
                    None => None,
                };
            } else if qop.is_none() && entry.starts_with("qop=") {
                qop = match get_non_unq_value(entry) {
                    Some(d) => Some(Qop::from_str(d)?),
                    None => None,
                };
            } else if response.is_none() && entry.starts_with(r#"response=""#) {
                response = get_unq_value(entry);
            } else if algorithm.is_none() && entry.starts_with("algorithm=") {
                algorithm = get_non_unq_value(entry);
            } else {
                debug!(?entry, "unexpected entry");
            }
        }

        let username = username.context("missing username")?;
        let realm = realm.context("missing realm")?;
        let nonce = nonce.context("missing nonce")?;
        let uri = uri.context("missing uri")?;
        let response = response.context("missing response")?;

        let qop = match qop {
            Some(d) => Some(d),
            None => {
                // for RFC 2069.
                return Ok(Self {
                    username: username.to_owned(),
                    realm: realm.to_owned(),
                    nonce: nonce.to_owned(),
                    uri: uri.to_owned(),
                    response: response.to_owned(),
                    ..Default::default()
                });
            }
        };

        let cnonce = cnonce.context("missing cnonce")?;
        let nc = nc.context("missing nc")?;

        // the algorithm directive's value is MD5 if unspecified.

        Ok(Self {
            username: username.to_owned(),
            realm: realm.to_owned(),
            nonce: nonce.to_owned(),
            uri: uri.to_owned(),
            cnonce: Some(cnonce.to_owned()),
            nc: Some(nc.to_owned()),
            qop,
            response: response.to_owned(),
            algorithm: algorithm.map(str::to_owned),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_digest_md5_auth() {
        let mut digest = md5::Md5::new();
        assert_eq!(
            digest_auth(
                &mut digest,
                "GET",
                "hoge",
                "fuga",
                "Secret Zone",
                "3GsPOCGsBQA=85dc6d369a4f2ee6b2d2d76b683559a62500570c",
                "/~68user/net/sample/http-auth-digest/secret.html",
                "MzE0OWMyOGE4MDc1NTgzM2QwNDQxYWViMjMzOTI3NGI=",
                "00000001",
            ),
            "c8aa77a1cd1a5837ada89294c08b1c0c"
        );
    }

    #[test]
    fn test_digest_md5_sess_auth() {
        let mut digest = md5::Md5::new();
        assert_eq!(
            digest_sess_auth(
                &mut digest,
                "GET",
                "foo",
                "bar",
                "Secret Zone",
                "8d86b8a5-0bbd-4283-9a9b-aad1e560b25f",
                "/md5_sess_auth",
                "ZmJjZWFhNTc5YTE4NWI0ZGQ1MDM1ODY2NDc3NDYwNmU=",
                "00000001",
            ),
            "d5fd3ecf15d83ec74597deb02df0ee62"
        )
    }

    #[test]
    fn test_digest_md5_auth_int() {
        let mut digest = md5::Md5::new();

        assert_eq!(
            digest_auth_int(
                &mut digest,
                "GET",
                "foo",
                "bar",
                "Secret Zone",
                "fc2410ed-a2e6-4009-b8d4-e87791fae2f1",
                "/md5_auth_int",
                "Njk1YjJiNzcyOTc3NGJkNjAwNGJjOWRkNWE4Y2Y0OWM=",
                "00000001",
                b"",
            ),
            "b75e3146715cd78f4ac9a43699b456db"
        );
    }

    #[test]
    fn test_digest_2069() {
        let mut digest = md5::Md5::new();
        assert_eq!(
            digest_2069(
                &mut digest,
                "GET",
                "foo",
                "bar",
                "Secret Zone",
                "238e70b6-cdb1-42b1-be74-fdf6d279760a",
                "/md5_auth_int",
            ),
            "8b9609731908f6c49aa3fd4d4df4ef0c"
        );
    }

    #[test]
    fn test_start_with() {
        let s = r#"Digest username="foo""#;

        assert!(s.starts_with("Digest "));
        assert_eq!(&s["Digest ".len()..], r#"username="foo""#);
    }

    #[test]
    fn digest_header_parameters() {
        let container = DigestHeaderParameters::from_str(
            r#"Digest username="My Name", realm="My Realm", nonce="NONCE123", uri="/path/to/secret", cnonce="CNONCE123", nc=00000001, qop=auth, response="RES123", algorithm=MD5"#,
        ).unwrap();

        assert_eq!(container.username, "My Name");
        assert_eq!(container.realm, "My Realm");
        assert_eq!(container.nonce, "NONCE123");
        assert_eq!(container.uri, "/path/to/secret");
        assert_eq!(container.cnonce, Some("CNONCE123".to_owned()));
        assert_eq!(container.nc, Some("00000001".to_owned()));
        assert_eq!(container.qop, Some(Qop::Auth));
        assert_eq!(container.response, "RES123");
        assert_eq!(container.algorithm, Some("MD5".to_owned()));
    }

    #[test]
    fn digest_header_parameters_rfc2069() {
        let container = DigestHeaderParameters::from_str(
            r#"Digest username="My Name", realm="My Realm", nonce="NONCE123", uri="/path/to/secret", response="RES123""#,
        ).unwrap();

        assert_eq!(container.username, "My Name");
        assert_eq!(container.realm, "My Realm");
        assert_eq!(container.nonce, "NONCE123");
        assert_eq!(container.uri, "/path/to/secret");
        assert_eq!(container.cnonce, None);
        assert_eq!(container.nc, None);
        assert_eq!(container.qop, None);
        assert_eq!(container.response, "RES123");
        assert_eq!(container.algorithm, None);
    }

    #[test]
    fn digest_header_parameters_rfc2069_qop() {
        let container = DigestHeaderParameters::from_str(
            r#"Digest username="My Name", realm="My Realm", nonce="NONCE123", uri="/path/to/secret", cnonce="CNONCE123", nc=00000001, response="RES123", algorithm=MD5"#,
        ).unwrap();

        assert_eq!(container.username, "My Name");
        assert_eq!(container.realm, "My Realm");
        assert_eq!(container.nonce, "NONCE123");
        assert_eq!(container.uri, "/path/to/secret");
        assert_eq!(container.cnonce, None);
        assert_eq!(container.nc, None);
        assert_eq!(container.qop, None);
        assert_eq!(container.response, "RES123");
        assert_eq!(container.algorithm, None);
    }

    #[test]
    fn digest_header_parameters_missing_digest() {
        let container = DigestHeaderParameters::from_str(
            r#"username="My Name", realm="My Realm", nonce="NONCE123", uri="/path/to/secret", cnonce="CNONCE123", nc=00000001, qop=auth, response="RES123", algorithm=MD5"#,
        );

        assert!(container.is_err(), "{:?}", container);
    }

    #[test]
    fn digest_header_parameters_missing_name() {
        let container = DigestHeaderParameters::from_str(
            r#"Digest realm="My Realm", nonce="NONCE123", uri="/path/to/secret", cnonce="CNONCE123", nc=00000001, qop=auth, response="RES123", algorithm=MD5"#,
        );

        assert!(container.is_err(), "{:?}", container);
    }

    #[test]
    fn digest_header_parameters_missing_realm() {
        let container = DigestHeaderParameters::from_str(
            r#"Digest username="My Name", nonce="NONCE123", uri="/path/to/secret", cnonce="CNONCE123", nc=00000001, qop=auth, response="RES123", algorithm=MD5"#,
        );

        assert!(container.is_err(), "{:?}", container);
    }

    #[test]
    fn digest_header_parameters_missing_nonce() {
        let container = DigestHeaderParameters::from_str(
            r#"Digest username="My Name", realm="My Realm", uri="/path/to/secret", cnonce="CNONCE123", nc=00000001, qop=auth, response="RES123", algorithm=MD5"#,
        );

        assert!(container.is_err(), "{:?}", container);
    }

    #[test]
    fn digest_header_parameters_missing_uri() {
        let container = DigestHeaderParameters::from_str(
            r#"Digest username="My Name", realm="My Realm", nonce="NONCE123", cnonce="CNONCE123", nc=00000001, qop=auth, response="RES123", algorithm=MD5"#,
        );

        assert!(container.is_err(), "{:?}", container);
    }

    #[test]
    fn digest_header_parameters_missing_response() {
        let container = DigestHeaderParameters::from_str(
            r#"Digest username="My Name", realm="My Realm", nonce="NONCE123", uri="/path/to/secret", cnonce="CNONCE123", nc=00000001, qop=auth, algorithm=MD5"#,
        );

        assert!(container.is_err(), "{:?}", container);
    }

    #[test]
    fn digest_header_parameters_qop_auth_int() {
        let container = DigestHeaderParameters::from_str(
            r#"Digest username="My Name", realm="My Realm", nonce="NONCE123", uri="/path/to/secret", cnonce="CNONCE123", nc=00000001, qop=auth-int, response="RES123", algorithm=MD5"#,
        ).unwrap();

        assert_eq!(container.username, "My Name");
        assert_eq!(container.realm, "My Realm");
        assert_eq!(container.nonce, "NONCE123");
        assert_eq!(container.uri, "/path/to/secret");
        assert_eq!(container.cnonce, Some("CNONCE123".to_owned()));
        assert_eq!(container.nc, Some("00000001".to_owned()));
        assert_eq!(container.qop, Some(Qop::AuthInt));
        assert_eq!(container.response, "RES123");
        assert_eq!(container.algorithm, Some("MD5".to_owned()));
    }

    #[test]
    fn digest_header_parameters_unexpected_qop() {
        let container = DigestHeaderParameters::from_str(
            r#"Digest username="My Name", realm="My Realm", nonce="NONCE123", uri="/path/to/secret", cnonce="CNONCE123", nc=00000001, qop=unexpected, response="RES123", algorithm=MD5"#,
        );

        assert!(container.is_err(), "{:?}", container);
    }

    #[test]
    fn digest_header_parameters_empty() {
        let container = DigestHeaderParameters::from_str(
            r#"Digest username="", realm="", nonce="", uri="", cnonce="", nc=, qop=auth, response="", algorithm="#,
        ).unwrap();

        assert_eq!(container.username, "");
        assert_eq!(container.realm, "");
        assert_eq!(container.nonce, "");
        assert_eq!(container.uri, "");
        assert_eq!(container.cnonce, Some("".to_owned()));
        assert_eq!(container.nc, Some("".to_owned()));
        assert_eq!(container.qop, Some(Qop::Auth));
        assert_eq!(container.response, "");
        assert_eq!(container.algorithm, Some("".to_owned()));
    }

    #[test]
    fn rspauth() {
        let mut digest = md5::Md5::new();
        assert_eq!(
            digest_auth(
                &mut digest,
                "",
                "hoge",
                "fuga",
                "Secret Zone",
                "QEmm1XWsBQA=f505faac0a748c0087dd80be2282e035588c00c8",
                "/~68user/net/sample/http-auth-digest/secret.html",
                "NDFhMTI4NzI2MDQyNTEzZDNiZDZhYjY3MmQ5MTM4NTU=",
                "00000001",
            ),
            "ab6d32825d50619c7c2b80106c493854"
        )
    }

    #[test]
    fn find_index() {
        let val = r#"cnonce="MWU0YzZlYjZkYzIyNGJiYmY0NzYzZGYxZjliNTQ1MWQ=""#;
        let index = val.find(r#"=""#).unwrap();
        assert_eq!(
            &val[index + 2..],
            r#"MWU0YzZlYjZkYzIyNGJiYmY0NzYzZGYxZjliNTQ1MWQ=""#
        );
    }

    #[test]
    fn get_unq_value_eq() {
        assert_eq!(
            get_unq_value(r#"cnonce="MWU0YzZlYjZkYzIyNGJiYmY0NzYzZGYxZjliNTQ1MWQ=""#),
            Some(r#"MWU0YzZlYjZkYzIyNGJiYmY0NzYzZGYxZjliNTQ1MWQ="#)
        );
    }
}
