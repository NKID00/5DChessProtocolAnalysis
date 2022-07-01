use server::ServerState;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{info, subscriber, Level};
use tracing_subscriber::FmtSubscriber;

pub mod datatype;
pub mod passcode;
pub mod server;

use crate::server::handle_connection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sub = FmtSubscriber::builder()
        .with_max_level(Level::TRACE)
        .finish();
    subscriber::set_global_default(sub)?;
    info!(
        "5dcserver {} ({}) [rustc {}]",
        env!("VERGEN_BUILD_SEMVER"),
        env!("VERGEN_GIT_SHA_SHORT"),
        env!("VERGEN_RUSTC_SEMVER")
    );

    let bind_addr = ("0.0.0.0", 39005);
    let listener = TcpListener::bind(bind_addr).await?;
    info!("listening on {}:{} ...", bind_addr.0, bind_addr.1);

    let state = Arc::new(ServerState::new());

    loop {
        let (stream, addr) = listener.accept().await?;
        info!("[{}:{}] Connected.", addr.ip(), addr.port());
        tokio::spawn(handle_connection(state.clone(), stream, addr));
    }
}
