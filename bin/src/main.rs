#![warn(clippy::pedantic, clippy::perf, missing_docs, clippy::missing_docs_in_private_items)]

//! TODO: This binary implementation is temporary. Don't keep it around.

use std::{error::Error, net::{TcpListener, TcpStream}, process::ExitCode, time::Duration};
use simplelog::*;
use threadpool::ThreadPool;
use once_cell::sync::Lazy;
use pollster::FutureExt as _;
#[macro_use]
extern crate log;


fn main() -> ExitCode {
    TermLogger::init(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Mixed,
        ColorChoice::Auto
    ).expect("no logger has been initialized yet");

    let res = inner_main();
    let Err(err) = res else { return ExitCode::SUCCESS; };
    error!("ENCOUNTERED FATAL ERROR");
    error!("{err}");
    return ExitCode::FAILURE;
}

fn inner_main() -> Result<(), Box<dyn Error>> {
    let mut thread_pool = ThreadPool::with_name("worker thread".into(), 4);

    let mut listener = TcpListener::bind("127.0.0.1:25565")?;

    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            let err = stream.unwrap_err();
            error!("TCP connection failed.");
            error!("{err}");
            continue;
        };

        thread_pool.execute(
            || handle_stream(stream).block_on()
        );
    }

    todo!()
}

async fn handle_stream(stream: TcpStream) {

}
