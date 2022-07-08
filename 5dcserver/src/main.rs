use futures::future::join_all;
use std::collections::VecDeque;
use std::env;
use std::error::Error;
use std::io::ErrorKind;
use std::process::exit;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::watch;
use tokio::{net::TcpListener, select};
use tracing::{info, subscriber, Level};
use tracing_subscriber::FmtSubscriber;

#[macro_use]
pub mod datatype;
pub mod server;

use server::{handle_connection, ServerState};

fn print_usage(arg0: &String) {
    println!();
    println!("usage: {} <CONFIG FILE>", arg0);
}

fn get_config<'a, T: toml::macros::Deserialize<'a>>(
    config: &toml::value::Map<String, toml::Value>,
    name: &str,
    default: T,
) -> T {
    match config.get(name) {
        Some(value) => match value.clone().try_into() {
            Ok(value) => value,
            _ => default,
        },
        None => default,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // banner
    println!(
        "5dcserver {} ({}) [rustc {}]",
        env!("VERGEN_BUILD_SEMVER"),
        env!("VERGEN_GIT_SHA_SHORT"),
        env!("VERGEN_RUSTC_SEMVER")
    );
    println!("Copyright (C) 2022 NKID00, licensed under AGPL-3.0-only");

    // parse args
    let args: Vec<String> = env::args().collect();
    if args.len() <= 1 {
        print_usage(&args[0]);
        exit(1);
    }

    // load config
    let config = match fs::read(&args[1]).await {
        Ok(config) => toml::from_str(String::from_utf8(config)?.as_str())?,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let config = toml::toml! {
                addr = "0.0.0.0"
                allow_reset_puzzle = false
                port = 39005
                trace = false
                variants = []
            };
            fs::write(&args[1], config.to_string()).await?;
            config
        }
        Err(e) => Err(e)?,
    }
    .try_into()
    .unwrap();

    // register tracing
    let trace = get_config(&config, "trace", false);
    let sub = FmtSubscriber::builder()
        .with_max_level(if cfg!(debug_assertions) || trace {
            Level::TRACE
        } else {
            Level::INFO
        })
        .finish();
    subscriber::set_global_default(sub)?;

    // handle ctrl-c
    let (running_tx, mut running_rx) = watch::channel(true);
    let state = Arc::new(ServerState::new());
    ctrlc::set_handler(move || {
        running_tx.send_if_modified(|running| {
            if *running {
                info!("Stopping ...");
                *running = false;
                true
            } else {
                false
            }
        });
    })?;

    // bind and listen for connections
    let addr = get_config(&config, "addr", "0.0.0.0");
    let port = get_config(&config, "port", 39005);
    let bind_addr = (addr, port);
    let listener = TcpListener::bind(bind_addr).await?;
    info!("listening on {}:{} ...", bind_addr.0, bind_addr.1);

    let mut handles = VecDeque::new();
    loop {
        select! {
            result = listener.accept() => {
                let (stream, addr) = result?;
                handles.push_back(tokio::spawn(handle_connection(state.clone(), stream, addr, running_rx.clone())));
            },
            result = running_rx.changed() => {
                join_all(handles).await;
                info!("Stopped.");
                break Ok(result?);
            }
        }
    }
}
