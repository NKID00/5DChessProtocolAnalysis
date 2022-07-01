use futures::{SinkExt, StreamExt};
use std::collections::{HashMap, VecDeque};
use std::io::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::{net::TcpStream, select};
use tokio_util::codec::Framed;
use tracing::error;

use crate::datatype::*;

#[derive(Debug)]
pub struct ServerState {
    match_id: Mutex<u64>,
    message_id: Mutex<u64>,
    matches: Mutex<VecDeque<InternalMatch>>,
    matches_map: Mutex<HashMap<i64, InternalMatch>>,
    public_matches: Mutex<VecDeque<PublicMatch>>,
    server_history_matches: Mutex<VecDeque<ServerHistoryMatch>>,
}

impl ServerState {
    pub fn new() -> ServerState {
        ServerState {
            match_id: Mutex::new(0),
            message_id: Mutex::new(0),
            matches: Mutex::new(VecDeque::<InternalMatch>::new()),
            matches_map: Mutex::new(HashMap::<i64, InternalMatch>::new()),
            public_matches: Mutex::new(VecDeque::<PublicMatch>::new()),
            server_history_matches: Mutex::new(VecDeque::<ServerHistoryMatch>::new()),
        }
    }
}

/* state machine of one connection:

Idle -> PublicMatchWaiting -> Playing -> Idle
Idle -> PrivateMatchWaiting -> Playing -> Idle
*/
pub enum ConnectionState {
    Idle,
    PublicMatchWaiting,
    PrivateMatchWaiting,
    Playing,
}

pub async fn handle_connection(
    server_state: Arc<ServerState>,
    stream: TcpStream,
    addr: SocketAddr,
) {
    let mut state = ConnectionState::Idle;
    let mut messages = Framed::new(stream, MessageCodec {});
    match handle_connection_main_loop(&server_state, &mut state, &addr, &mut messages).await {
        Ok(()) => {}
        Err(e) => {
            error!("[{}:{}] {}", addr.ip(), addr.port(), e);
        }
    }
    // TODO: clean resources, remove match from public_matches, etc.
    error!("[{}:{}] Disconnected.", addr.ip(), addr.port());
}

pub async fn handle_connection_main_loop(
    server_state: &Arc<ServerState>,
    state: &mut ConnectionState,
    addr: &SocketAddr,
    messages: &mut Framed<TcpStream, MessageCodec>,
) -> Result<()> {
    loop {
        match messages.next().await {
            Some(Ok(message)) => {
                match state {
                    ConnectionState::Idle => {
                        handle_message_idle(server_state, state, addr, messages, message).await?
                    }
                    ConnectionState::PublicMatchWaiting => {}
                    ConnectionState::PrivateMatchWaiting => {}
                    ConnectionState::Playing => {}
                };
                messages.flush().await?;
            }
            Some(Err(e)) => return Err(e),
            None => return Ok(()), // disconnected
        }
    }
}

async fn handle_message_idle(
    server_state: &Arc<ServerState>,
    state: &mut ConnectionState,
    addr: &SocketAddr,
    messages: &mut Framed<TcpStream, MessageCodec>,
    message: Message,
) -> Result<()> {
    match message {
        Message::C2SGreet(_body) => {
            messages.feed(Message::S2CGreet).await?;
        }
        Message::C2SMatchCreateOrJoin(body) => {
            if body.passcode < 0 {
                // join match
                let matches_map = server_state.matches_map.lock().await;
                match matches_map.get(&body.passcode) {
                    Some(_) => todo!(),
                    None => {
                        messages
                            .feed(Message::S2CMatchCreateOrJoinResult(S2CMatchCreateOrJoinResultBody { color: todo!(), clock: todo!(), variant: todo!(), visibility: todo!(), passcode: todo!() }))
                            .await?
                    }
                }
            } else {
                // create match
                let internal_match = InternalMatch {
                    color: body.color,
                    clock: body.clock,
                    variant: body.variant,
                    visibility: body.visibility,
                    passcode: body.passcode,
                };
                match body.visibility {
                    Visibility::Public => {
                        *state = ConnectionState::PublicMatchWaiting;
                    }
                    Visibility::Private => {
                        *state = ConnectionState::PrivateMatchWaiting;
                    }
                }
            }
        }
        Message::C2SMatchCancel => {
            messages
                .feed(Message::S2CMatchCancelResult(S2CMatchCancelResultBody { result: 0 }))
                .await?;
        }
        Message::C2SMatchListRequest => {
            //messages.feed().await;
        }
        _ => {
            return error_invalid_data!("Invalid message type {:?} at state Idle.", message);
        }
    }
    Ok(())
}
