use clap::Parser;
use futures::StreamExt;
use rust_myscript::prelude::*;
use std::sync::Arc;
use tracing::info;

#[derive(Parser)]
struct Opt {
    #[clap(short, long)]
    jobs: Option<usize>,

    #[clap(short, long)]
    heavy: usize,

    #[clap(long, default_value = "1000000")]
    heavy_weight: usize,

    #[clap(short, long)]
    light: usize,

    #[clap(long, default_value = "1000")]
    light_weight: usize,
}

#[tokio::main]
async fn main() -> Fallible<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    info!("Hello");

    let opt: Opt = Opt::parse();

    println!("work 1-0");

    let works = create_1_0(opt.heavy, opt.heavy_weight, opt.light, opt.light_weight);
    let jobs = opt.jobs.unwrap_or_else(num_cpus::get);
    let thread_pool_like_start = std::time::SystemTime::now();
    thread_pool_like(jobs, works).await;
    let thread_pool_like_end = std::time::SystemTime::now();

    let works = create_1_0(opt.heavy, opt.heavy_weight, opt.light, opt.light_weight);
    let semaphore_type_start = std::time::SystemTime::now();
    semaphore_type(jobs, works).await;
    let semaphore_type_end = std::time::SystemTime::now();

    println!(
        "thread_pool_like: {:?}",
        thread_pool_like_end
            .duration_since(thread_pool_like_start)
            .unwrap()
    );
    println!(
        "semaphore_type:   {:?}",
        semaphore_type_end
            .duration_since(semaphore_type_start)
            .unwrap()
    );

    info!("Bye");

    Ok(())
}

async fn thread_pool_like(jobs: usize, mut works: Vec<impl Fn() -> usize + Send + 'static>) {
    let window = works.len() / jobs;
    let futs = futures::stream::FuturesUnordered::new();
    for index in 0..jobs {
        let entries = if index == jobs - 1 {
            works.drain(..).collect::<Vec<_>>()
        } else {
            works.drain(0..window).collect::<Vec<_>>()
        };

        let fut = tokio::task::spawn(async move {
            let mut ret = Vec::with_capacity(entries.len());
            for w in entries {
                ret.push(w());
            }
            ret
        });
        futs.push(fut);
    }
    futs.collect::<Vec<_>>().await;
}

async fn semaphore_type(jobs: usize, works: Vec<impl Fn() -> usize + Send + 'static>) {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(jobs));
    let futs = futures::stream::FuturesUnordered::new();
    for w in works {
        let semaphore = semaphore.clone();
        let fut = tokio::task::spawn(async move {
            let _lock = semaphore.acquire().await;
            w()
        });
        futs.push(fut);
    }
    futs.collect::<Vec<_>>().await;
}

fn create_work(count: usize) -> impl Fn() -> usize {
    move || {
        let mut a = 0;
        for i in 0..count {
            a = nothing(i);
        }
        a
    }
}

fn nothing(i: usize) -> usize {
    i
}

fn create_1_0(
    heavy: usize,
    heavy_weight: usize,
    light: usize,
    light_weight: usize,
) -> Vec<impl Fn() -> usize + Send + 'static> {
    let mut works = vec![];
    for _ in 0..heavy {
        works.push(create_work(heavy_weight));
    }

    for _ in 0..light {
        works.push(create_work(light_weight));
    }

    works
}
