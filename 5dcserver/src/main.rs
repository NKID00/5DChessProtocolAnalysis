use futures::future::join_all;
use std::collections::{HashSet, VecDeque};
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
use datatype::*;

fn print_usage(arg0: &String) {
    println!();
    println!("usage: {} <CONFIG FILE>", arg0);
}

fn get_config<'a, T: toml::macros::Deserialize<'a>>(
    config: &toml::value::Table,
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
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_GIT_SHA"),
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

    // init server state
    let allow_reset_puzzle = get_config(&config, "allow_reset_puzzle", false);
    let variants = get_config(&config, "variants", toml::value::Array::new());
    let variants = {
        let mut variants_set = HashSet::new();
        if variants.len() == 0 {
            for i in 1..46 {
                variants_set.insert(try_i64_to_enum(i)?);
            }
        } else {
            for i in variants {
                variants_set.insert(try_i64_to_enum(i.as_integer().unwrap())?);
            }
        }
        variants_set
    };
    let state = Arc::new(ServerState::new(allow_reset_puzzle, variants));

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
