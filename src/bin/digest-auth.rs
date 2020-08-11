use anyhow::Context;
use log::{debug, info, warn};
use md5::Digest;
use std::str::FromStr;
use structopt::StructOpt;
use warp::http::header;
use warp::http::StatusCode;
use warp::Filter;

#[derive(Debug, StructOpt)]
struct Opt {
    #[structopt(short, long)]
    realm: String,

    #[structopt(short, long)]
    port: u16,
}

#[derive(Debug, serde::Deserialize)]
struct DigestQuery {
    #[serde(rename = "valid")]
    ignore_cert_error: Option<String>,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();

    info!("Hello");

    let opt: Opt = Opt::from_args();

    let filter_auth_header = warp::header::optional("Authorization");
    let realm = std::sync::Arc::new(opt.realm);

    let digest_2069_realm = realm.clone();
    let digest_2069 = warp::get()
        .and(warp::path!("digest_2069"))
        .and(filter_auth_header)
        .and(warp::query::<DigestQuery>())
        .map(
            move |header_authorization: Option<String>, digest_query: DigestQuery| {
                let realm = digest_2069_realm.clone();
                info!(
                    "rfc2069 header: {:?}, query: {:?}",
                    header_authorization, digest_query
                );

                let www_auth = warp::http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(
                        header::WWW_AUTHENTICATE,
                        format!(
                            r#"Digest realm="{}", nonce="{}", algorithm=MD5"#,
                            realm,
                            uuid::Uuid::new_v4().to_string()
                        ),
                    )
                    .body("ng".to_owned());

                let header_authorization = match header_authorization {
                    Some(data) => match DigestHeaderParameters::from_str(&data) {
                        Ok(d) => d,
                        Err(e) => {
                            info!("{:?}", e);
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
                    "bar",
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

    let md5_auth_realm = realm.clone();
    let md5_auth = warp::get()
        .and(warp::path!("md5_auth"))
        .and(filter_auth_header)
        .and(warp::query::<DigestQuery>())
        .map(
            move |header_authorization: Option<String>, digest_query: DigestQuery| {
                let realm = md5_auth_realm.clone();
                info!(
                    "md5_auth header: {:?}, query: {:?}",
                    header_authorization, digest_query
                );

                let www_auth = warp::http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(
                        header::WWW_AUTHENTICATE,
                        format!(
                            r#"Digest realm="{}", nonce="{}", algorithm=MD5, qop="auth""#,
                            realm,
                            uuid::Uuid::new_v4().to_string()
                        ),
                    )
                    .body("ng".to_owned());

                let header_authorization = match header_authorization {
                    Some(data) => match DigestHeaderParameters::from_str(&data) {
                        Ok(d) => d,
                        Err(e) => {
                            info!("{:?}", e);
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

                let mut digest = md5::Md5::new();
                let calc_hash = digest_auth(
                    &mut digest,
                    "GET",
                    &header_authorization.username,
                    "bar",
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
                                digest_auth(
                                    &mut digest,
                                    "",
                                    &header_authorization.username,
                                    "bar",
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

    let md5_sess_auth_realm = realm.clone();
    let md5_sess_auth = warp::get()
        .and(warp::path!("md5_sess_auth"))
        .and(filter_auth_header)
        .and(warp::query::<DigestQuery>())
        .map(
            move |header_authorization: Option<String>, digest_query: DigestQuery| {
                let realm = md5_sess_auth_realm.clone();
                info!(
                    "md5_sess_auth header: {:?}, query: {:?}",
                    header_authorization, digest_query
                );

                let www_auth = warp::http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(
                        header::WWW_AUTHENTICATE,
                        format!(
                            r#"Digest realm="{}", nonce="{}", algorithm=MD5-sess, qop="auth""#,
                            realm,
                            uuid::Uuid::new_v4().to_string()
                        ),
                    )
                    .body("ng".to_owned());

                let header_authorization = match header_authorization {
                    Some(data) => match DigestHeaderParameters::from_str(&data) {
                        Ok(d) => d,
                        Err(e) => {
                            info!("{:?}", e);
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

                let mut digest = md5::Md5::new();
                let calc_hash = digest_sess_auth(
                    &mut digest,
                    "GET",
                    &header_authorization.username,
                    "bar",
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
                                digest_sess_auth(
                                    &mut digest,
                                    "",
                                    &header_authorization.username,
                                    "bar",
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

    let md5_auth_int_realm = realm.clone();
    let md5_auth_int = warp::get()
        .and(warp::path!("md5_auth_int"))
        .and(filter_auth_header)
        .and(warp::query::<DigestQuery>())
        .and(warp::body::bytes())
        .map(
            move |header_authorization: Option<String>,
                  digest_query: DigestQuery,
                  body: bytes::Bytes| {
                let realm = md5_auth_int_realm.clone();
                info!(
                    "md5_auth_int (GET) header: {:?}, query: {:?}, body: {:?}",
                    header_authorization,
                    digest_query,
                    String::from_utf8_lossy(&body)
                );

                let www_auth = warp::http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(
                        header::WWW_AUTHENTICATE,
                        format!(
                            r#"Digest realm="{}", nonce="{}", algorithm=MD5, qop="auth-int""#,
                            realm,
                            uuid::Uuid::new_v4().to_string()
                        ),
                    )
                    .body("".to_owned());

                let header_authorization = match header_authorization {
                    Some(data) => match DigestHeaderParameters::from_str(&data) {
                        Ok(d) => d,
                        Err(e) => {
                            info!("{:?}", e);
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

                let mut digest = md5::Md5::new();
                if !body.is_empty() {
                    digest.update(body);
                }
                let body_hash = HexFormat(digest.finalize_reset().as_slice()).to_string();
                let calc_hash = digest_auth_int(
                    &mut digest,
                    "GET",
                    &header_authorization.username,
                    "bar",
                    &header_authorization.realm,
                    &header_authorization.nonce,
                    &header_authorization.uri,
                    cnonce,
                    nc,
                    &body_hash,
                );

                if calc_hash == header_authorization.response {
                    info!("accept");
                    warp::http::Response::builder()
                        .header(
                            "Authentication-Info",
                            format!(
                                r#"rspauth="{}", cnonce="{}", nc={}, qop={}"#,
                                digest_auth_int(
                                    &mut digest,
                                    "",
                                    &header_authorization.username,
                                    "bar",
                                    &header_authorization.realm,
                                    &header_authorization.nonce,
                                    &header_authorization.uri,
                                    cnonce,
                                    nc,
                                    &body_hash
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

    let md5_auth_int_post_realm = realm.clone();
    let md5_auth_int_post = warp::post()
        .and(warp::path!("md5_auth_int"))
        .and(filter_auth_header)
        .and(warp::query::<DigestQuery>())
        .and(warp::body::bytes())
        .map(
            move |header_authorization: Option<String>,
                  digest_query: DigestQuery,
                  body: bytes::Bytes| {
                let realm = md5_auth_int_post_realm.clone();
                info!(
                    "md5_auth_int (POST) header: {:?}, query: {:?} body {:?}",
                    header_authorization,
                    digest_query,
                    String::from_utf8_lossy(&body)
                );

                let www_auth = warp::http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(
                        header::WWW_AUTHENTICATE,
                        format!(
                            r#"Digest realm="{}", nonce="{}", algorithm=MD5, qop="auth-int""#,
                            realm,
                            uuid::Uuid::new_v4().to_string()
                        ),
                    )
                    .body("".to_owned());

                let header_authorization = match header_authorization {
                    Some(data) => match DigestHeaderParameters::from_str(&data) {
                        Ok(d) => d,
                        Err(e) => {
                            info!("{:?}", e);
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

                let mut digest = md5::Md5::new();
                if !body.is_empty() {
                    digest.update(body);
                }
                let body_hash = HexFormat(digest.finalize_reset().as_slice()).to_string();
                let calc_hash = digest_auth_int(
                    &mut digest,
                    "POST",
                    &header_authorization.username,
                    "bar",
                    &header_authorization.realm,
                    &header_authorization.nonce,
                    &header_authorization.uri,
                    cnonce,
                    nc,
                    &body_hash,
                );

                if calc_hash == header_authorization.response {
                    info!("accept");
                    warp::http::Response::builder()
                        .header(
                            "Authentication-Info",
                            format!(
                                r#"rspauth="{}", cnonce="{}", nc={}, qop={}"#,
                                digest_auth_int(
                                    &mut digest,
                                    "",
                                    &header_authorization.username,
                                    "bar",
                                    &header_authorization.realm,
                                    &header_authorization.nonce,
                                    &header_authorization.uri,
                                    cnonce,
                                    nc,
                                    &body_hash
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

fn digest_auth<D: Digest>(
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
    digest.reset();

    update_parameters_to_digest(digest, [username, realm, password]);
    let a1 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [http_method, uri]);
    let a2 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [&a1, nonce, nc, cnonce, "auth", &a2]);
    HexFormat(digest.finalize_reset().as_slice()).to_string()
}

fn digest_sess_auth<D: Digest>(
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
    digest.reset();

    update_parameters_to_digest(digest, [username, realm, password]);
    let a1_prefix = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [&a1_prefix, nonce, cnonce]);
    let a1 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [http_method, uri]);
    let a2 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [&a1, nonce, nc, cnonce, "auth", &a2]);
    HexFormat(digest.finalize_reset().as_slice()).to_string()
}

fn digest_2069<D: Digest>(
    digest: &mut D,
    http_method: &str,
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    uri: &str,
) -> String {
    digest.reset();

    update_parameters_to_digest(digest, [username, realm, password]);
    let a1 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [http_method, uri]);
    let a2 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    update_parameters_to_digest(digest, [&a1, nonce, &a2]);
    HexFormat(digest.finalize_reset().as_slice()).to_string()
}

fn digest_auth_int<D: Digest>(
    digest: &mut D,
    http_method: &str,
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    uri: &str,
    cnonce: &str,
    nc: &str,
    body_hash: &str,
) -> String {
    digest.reset();

    update_parameters_to_digest(digest, [username, realm, password]);
    let a1 = HexFormat(digest.finalize_reset().as_slice()).to_string();

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
    qop: Option<String>,
    response: String,
    algorithm: Option<String>,
}

fn get_non_unq_value(segment: &str) -> Option<&str> {
    // orig: abc123=value
    match segment.find('=') {
        Some(index) => Some(&segment[index + 1..segment.len()]),
        None => {
            warn!("unexpected format: {}", segment);
            None
        }
    }
}

fn get_unq_value(segment: &str) -> Option<&str> {
    // orig: abc123="value"
    match segment.find(r#"=""#) {
        Some(index) => Some(&segment[index + 2..segment.len() - 1]),
        None => {
            warn!("unexpected format: {}", segment);
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
            } else if qop.is_none() && entry.starts_with("qop=") {
                qop = get_non_unq_value(entry);
            } else if response.is_none() && entry.starts_with(r#"response=""#) {
                response = get_unq_value(entry);
            } else if algorithm.is_none() && entry.starts_with("algorithm=") {
                algorithm = get_non_unq_value(entry);
            } else {
                debug!("unexpected entry: {:?}", entry);
            }
        }

        let username = username.context("missing username")?;
        let realm = realm.context("missing realm")?;
        let nonce = nonce.context("missing nonce")?;
        let uri = uri.context("missing uri")?;
        let response = response.context("missing response")?;

        let qop = match qop {
            Some(d) => d,
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

        // TODO: use enum.
        if qop != "auth" && qop != "auth-int" {
            anyhow::bail!("unexpected qop: {}", qop);
        }

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
            qop: Some(qop.to_owned()),
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
        let md5sum_empty = HexFormat(digest.finalize_reset().as_slice()).to_string();

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
                &md5sum_empty,
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
        assert_eq!(container.qop, Some("auth".to_owned()));
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
        assert_eq!(container.qop, Some("auth-int".to_owned()));
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
        assert_eq!(container.qop, Some("auth".to_owned()));
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
