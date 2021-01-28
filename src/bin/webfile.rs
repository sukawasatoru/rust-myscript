use std::path::PathBuf;
use structopt::StructOpt;

#[derive(StructOpt)]
struct Opt {
    /// Directory to root
    #[structopt(parse(from_os_str), default_value = ".")]
    dir: PathBuf,

    /// Port number
    #[structopt(short, long, default_value = "38080")]
    port: u16,
}

#[tokio::main]
async fn main() {
    let opt: Opt = Opt::from_args();

    warp::serve(warp::fs::dir(opt.dir))
        .run(([0, 0, 0, 0], opt.port))
        .await;
}
