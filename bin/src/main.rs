#![warn(clippy::pedantic, clippy::perf, missing_docs, clippy::missing_docs_in_private_items)]

//! TODO: This binary implementation is temporary. Don't keep it around.

mod networking;

use std::{
    error::Error,
    process::ExitCode
};
#[macro_use]
extern crate log;

#[tokio::main]
async fn main() -> ExitCode {
    simplelog::TermLogger::init(
        if cfg!(debug_assertions) {
            simplelog::LevelFilter::Trace
        } else {
            simplelog::LevelFilter::Info
        },
        simplelog::Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto
    ).expect("no logger has been initialized yet");

    let res: Result<(), Box<dyn Error>> = networking::start(todo!()).await.map_err(|e| e.into());
    let Err(err) = res else { return ExitCode::SUCCESS; };
    error!("~~~ ENCOUNTERED FATAL ERROR ~~~");
    error!("{err}");
    ExitCode::FAILURE
}
