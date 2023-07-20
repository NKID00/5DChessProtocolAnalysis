use anyhow::Result;
use futures::future::join_all;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use serde::Deserialize;
use std::collections::VecDeque;
use std::env;
use std::fs;
use std::io::ErrorKind;
use std::process::exit;
use std::sync::Arc;
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

#[derive(Deserialize)]
struct Config {
    addr: Vec<String>,
    port: u16,
    trace: bool,

    ban_public_match: bool,
    ban_private_match: bool,
    ban_reset_puzzle: bool,
    ban_variant: Vec<i64>,

    limit_concurrent_match: usize,
    limit_public_waiting: usize,
    limit_connection_duration: u64,
    limit_message_length: usize,
}

fn get_config<'a, T: toml::macros::Deserialize<'a>>(
    config: &toml::Table,
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
async fn main() -> Result<()> {
    // banner
    println!(
        "5dcserver {} ({}) [rustc {}]",
        env!("VERGEN_BUILD_SEMVER"),
        match option_env!("VERGEN_GIT_SHA_SHORT") {
            Some(s) => s,
            None => "unknown rev",
        },
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
    let config: Config = match fs::read_to_string(&args[1]) {
        Ok(config) => toml::from_str(config.as_str())?,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let default_config = include_str!("5dcserver.toml.example");
            fs::write(&args[1], default_config)?;
            toml::from_str(default_config)?
        }
        Err(e) => Err(e)?,
    };

    // register tracing
    let trace = config.trace;
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
    let listeners = Vec::new();
    for addr in config.addr {
        listeners.push(TcpListener::bind((addr, config.port)).await?);
        info!("listening on {}:{} ...", addr, config.port);
    }

    // init server state
    let state = Arc::new(ServerState::new(config)?);

    let mut handles = VecDeque::new();
    loop {
        let futures: FuturesUnordered<_> = listeners
            .into_iter()
            .map(|listener| listener.accept())
            .collect();
        select! {
            result = futures.next() => {
                let (stream, addr) = result.unwrap()?;
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
