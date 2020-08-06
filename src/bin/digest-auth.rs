use log::info;
use md5::Digest;
use warp::http::header;
use warp::http::StatusCode;
use warp::Filter;

struct HexFormat<'a>(&'a [u8]);

impl<'a> std::fmt::Display for HexFormat<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }

        let ret_first = write!(f, "{:02x?}", self.0[0]);
        if ret_first.is_err() {
            return ret_first;
        }

        for entry in &self.0[1..self.0.len()] {
            let entry_ret = write!(f, "{:02x?}", entry);
            if entry_ret.is_err() {
                return entry_ret;
            }
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    env_logger::init();

    info!("Hello");

    let md5_auth = warp::get()
        .and(warp::path!("md5_auth"))
        .and(warp::header::optional(header::AUTHORIZATION.as_str()))
        .map(|header_authorization: Option<String>| {
            if let None = header_authorization {
                return warp::http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(header::WWW_AUTHENTICATE, r#"Digest realm="Secret Zone", nonce="FQxLDzasBQA=3c0d056f2ea16d54c609713334a97de46c017ba8", algorithm=MD5, qop="auth""#.to_owned())
                    .body("".to_owned());
            }
            warp::http::Response::builder().body("ok\n".to_owned())
        });

    warp::serve(md5_auth).run(([0, 0, 0, 0], 59501)).await;

    info!("Bye");
    Ok(())
}

// algorithm-sess: Hex("${Hex(md.digest("$username:$realm:$password".as_bytes()))}:$nonce:$cnonce".as_bytes())
// auth-int: Hex(md.digest("$httpMethod:$uri:$entityBody".as_bytes()))

fn digest_md5_auth(
    http_method: &str,
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    uri: &str,
    cnonce: &str,
    nc: &str,
    qop: &str,
) -> String {
    let mut digest = md5::Md5::new();

    digest.update(format!("{}:{}:{}", username, realm, password).as_bytes());
    let a1 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    digest.update(format!("{}:{}", http_method, uri).as_bytes());
    let a2 = HexFormat(digest.finalize_reset().as_slice()).to_string();

    digest.update(format!("{}:{}:{}:{}:{}:{}", a1, nonce, nc, cnonce, qop, a2).as_bytes());
    HexFormat(digest.finalize().as_slice()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_digest_md5_auth() {
        assert_eq!(
            digest_md5_auth(
                "GET",
                "hoge",
                "fuga",
                "Secret Zone",
                "3GsPOCGsBQA=85dc6d369a4f2ee6b2d2d76b683559a62500570c",
                "/~68user/net/sample/http-auth-digest/secret.html",
                "MzE0OWMyOGE4MDc1NTgzM2QwNDQxYWViMjMzOTI3NGI=",
                "00000001",
                "auth"
            ),
            "c8aa77a1cd1a5837ada89294c08b1c0c"
        );
    }
}
