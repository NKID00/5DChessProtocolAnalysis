use anyhow::Result;
use futures::future::join_all;
use indoc::indoc;
use std::collections::{HashSet, VecDeque};
use std::env;
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

use datatype::*;
use server::{handle_connection, ServerState};

fn print_usage(arg0: &String) {
    println!();
    println!("usage: {} <CONFIG FILE>", arg0);
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
            None => "unknown rev"
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
    let config = match fs::read(&args[1]).await {
        Ok(config) => toml::from_str(String::from_utf8(config)?.as_str())?,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let default_config = indoc! {"
                addr = [\"0.0.0.0\", \"::\"]  # Bind address
                port = 39005  # Bind port, official server uses 39005
                trace = true  # Print detailed debug information
                
                ban_public_match = false  # Ban public matches (allow private matches only)
                ban_private_match = false  # Ban private matches (allow public matches only)
                ban_reset_puzzle = true  # Ban illegal game-resetting messages
                ban_variant = []  # IDs of banned variants, see the variant list
                
                limit_concurrent_match = 2000  # maximum number of matches
                limit_public_waiting = 100  # maximum number of public waiting matches
                limit_connection_duration = 259200  # maximum duration of a client connection in seconds
                limit_message_length = 4096  # maximum length of a network packet in bytes, must be >= 1008
            "};
            fs::write(&args[1], default_config).await?;
            toml::from_str(default_config)?
        }
        Err(e) => Err(e)?,
    };

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
