#![warn(clippy::pedantic, clippy::perf, missing_docs, clippy::missing_docs_in_private_items)]

use std::{net::TcpStream, process::ExitCode};
use simplelog::*;
#[macro_use]
use log::*;


#[tokio::main]
async fn main() -> ExitCode {
    let res = inner_main().await;
    let Err(err) = res else { return ExitCode::SUCCESS; };
    error!("ENCOUNTERED FATAL ERROR");
    error!("{err}");
    return ExitCode::FAILURE;
}