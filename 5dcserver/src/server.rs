use std::collections::{HashMap, VecDeque};
use std::io::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::{net::TcpStream, select};
use tracing::error;

use crate::datatype::*;

#[derive(Debug)]
pub struct ServerState {
    match_id: Mutex<u64>,
    message_id: Mutex<u64>,
    matches: Mutex<HashMap<i64, InternalMatch>>,
    public_matches: Mutex<VecDeque<PublicMatch>>,
    server_history_matches: Mutex<VecDeque<ServerHistoryMatch>>,
}

impl ServerState {
    pub fn new() -> ServerState {
        ServerState {
            match_id: Mutex::new(0),
            message_id: Mutex::new(0),
            matches: Mutex::new(HashMap::<i64, InternalMatch>::new()),
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
    let mut messages = MessageIO::new(stream);
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
    messages: &mut MessageIO,
) -> Result<()> {
    loop {
        match messages.get().await {
            Ok(message) => {
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
            Err(e) => return Err(e),
        }
    }
}

async fn handle_message_idle(
    server_state: &Arc<ServerState>,
    state: &mut ConnectionState,
    addr: &SocketAddr,
    messages: &mut MessageIO,
    message: Message,
) -> Result<()> {
    match message {
        Message::C2SGreet(_body) => {
            messages.put(Message::S2CGreet).await?;
        }
        Message::C2SMatchCreateOrJoin(body) => {
            if body.passcode >= 0 {
                // join match
                let matches_map = server_state.matches.lock().await;
                match matches_map.get(&body.passcode) {
                    Some(m) => {
                        // match found
                        messages
                            .put(Message::S2CMatchCreateOrJoinResult(
                                S2CMatchCreateOrJoinResultBody::Success(
                                    S2CMatchCreateOrJoinResultSuccessBody {
                                        color: m.color,
                                        clock: m.clock,
                                        variant: m.variant,
                                        visibility: m.visibility,
                                        passcode: m.passcode,
                                    },
                                ),
                            ))
                            .await?;
                    }
                    None => {
                        // match not found
                        messages
                            .put(Message::S2CMatchCreateOrJoinResult(
                                S2CMatchCreateOrJoinResultBody::Failed,
                            ))
                            .await?;
                    }
                }
            } else {
                // create match
                let passcode =
                    generate_random_passcode_internal_with_exceptions(&server_state.matches).await;
                server_state.matches.lock().await.insert(
                    passcode,
                    InternalMatch {
                        color: body.color,
                        clock: body.clock,
                        variant: body.variant,
                        visibility: body.visibility,
                        passcode,
                    },
                );
                match body.visibility {
                    Visibility::Public => {
                        server_state
                            .public_matches
                            .lock()
                            .await
                            .push_back(PublicMatch {
                                color: body.color,
                                clock: body.clock,
                                variant: body.variant,
                                passcode,
                            });
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
                .put(Message::S2CMatchCancelResult(
                    S2CMatchCancelResultBody::Failed,
                ))
                .await?;
        }
        Message::C2SMatchListRequest => {
            //messages.feed().await;
        }
        _ => {
            return err_invalid_data!("Invalid message type {:?} at state Idle.", message);
        }
    }
    Ok(())
}
