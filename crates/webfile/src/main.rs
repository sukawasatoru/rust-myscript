use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use clap::Parser;
use rust_myscript::prelude::*;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::services::ServeDir;

#[derive(Parser)]
struct Opt {
    /// Port number
    #[arg(short, long, default_value = "38080")]
    port: u16,

    /// Path of the fullchain.pem / This flag requires "key-path" flag
    #[arg(short, long)]
    cert_path: Option<PathBuf>,

    /// Path of the privkey.pem / This flag requires "cert-path" flag
    #[arg(short, long)]
    key_path: Option<PathBuf>,

    /// Directory to root
    #[arg(default_value = ".")]
    dir: PathBuf,
}

#[tokio::main]
async fn main() -> Fallible<()> {
    let opt: Opt = Opt::parse();

    let cert_args: Option<(PathBuf, PathBuf)> = match (opt.cert_path, opt.key_path) {
        (Some(cert_path), Some(key_path)) => Some((cert_path, key_path)),
        (None, None) => None,
        _ => {
            anyhow::bail!(
                "The cert argument requires pair of the \"--cert-path\" and \"--key-path\""
            )
        }
    };

    let app = Router::new().fallback_service(ServeDir::new(opt.dir));
    let addr = SocketAddr::from(([0, 0, 0, 0], opt.port));

    if let Some((cert_path, key_path)) = cert_args {
        let config = RustlsConfig::from_pem_file(cert_path, key_path).await?;
        axum_server::bind_rustls(addr, config)
            .serve(app.into_make_service())
            .await?;
    } else {
        axum_server::bind(addr)
            .serve(app.into_make_service())
            .await?;
    }

    Ok(())
}
