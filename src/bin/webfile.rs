use clap::Parser;
use rust_myscript::prelude::*;
use std::path::PathBuf;

#[derive(Parser)]
struct Opt {
    /// Port number
    #[clap(short, long, default_value = "38080")]
    port: u16,

    /// Path of the fullchain.pem / This flag requires "key-path" flag
    #[clap(short, long, parse(from_os_str))]
    cert_path: Option<PathBuf>,

    /// Path of the privkey.pem / This flag requires "cert-path" flag
    #[clap(short, long, parse(from_os_str))]
    key_path: Option<PathBuf>,

    /// Directory to root
    #[clap(parse(from_os_str), default_value = ".")]
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

    let addr = ([0, 0, 0, 0], opt.port);

    if let Some((cert_path, key_path)) = cert_args {
        warp::serve(warp::fs::dir(opt.dir))
            .tls()
            .cert_path(cert_path)
            .key_path(key_path)
            .run(addr)
            .await;
    } else {
        warp::serve(warp::fs::dir(opt.dir)).run(addr).await;
    }

    Ok(())
}
