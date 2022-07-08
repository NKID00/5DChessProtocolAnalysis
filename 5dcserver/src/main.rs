use futures::future::join_all;
use std::collections::VecDeque;
use std::error::Error;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::{net::TcpListener, select};
use tracing::{info, subscriber, Level};
use tracing_subscriber::FmtSubscriber;

#[macro_use]
pub mod datatype;
pub mod server;

use server::{handle_connection, ServerState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!(
        "5dcserver {} ({}) [rustc {}]",
        env!("VERGEN_BUILD_SEMVER"),
        env!("VERGEN_GIT_SHA_SHORT"),
        env!("VERGEN_RUSTC_SEMVER")
    );
    println!("Copyright (C) 2022 NKID00, licensed under AGPL-3.0-only");

    let sub = FmtSubscriber::builder()
        .with_max_level(if cfg!(debug_assertions) {
            Level::TRACE
        } else {
            Level::INFO
        })
        .finish();
    subscriber::set_global_default(sub)?;

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

    let bind_addr = ("0.0.0.0", 39005);
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
