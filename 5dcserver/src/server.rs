use byteorder::{ByteOrder, LittleEndian};
use bytes::{Bytes, BytesMut};
use futures::{SinkExt, StreamExt};
use std::collections::VecDeque;
use std::io::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::{net::TcpStream, select};
use tokio_util::codec::Framed;
use tracing::error;

use crate::datatype::*;
use crate::passcode::generate_random_passcode_internal;

#[derive(Debug)]
pub struct ServerState {
    match_id: Mutex<u64>,
    message_id: Mutex<u64>,
    matches: Mutex<VecDeque<Match>>,
    public_matches: Mutex<VecDeque<PublicMatch>>,
    server_history_matches: Mutex<VecDeque<ServerHistoryMatch>>,
}

impl ServerState {
    pub fn new() -> ServerState {
        ServerState {
            match_id: Mutex::new(0),
            message_id: Mutex::new(0),
            matches: Mutex::new(std::collections::VecDeque::<Match>::new()),
            public_matches: Mutex::new(std::collections::VecDeque::<PublicMatch>::new()),
            server_history_matches: Mutex::new(
                std::collections::VecDeque::<ServerHistoryMatch>::new(),
            ),
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
    mut stream: TcpStream,
    addr: SocketAddr,
) {
    let mut state = ConnectionState::Idle;
    let mut messages = Framed::new(stream, MessageCodec {});
    loop {
        match match state {
            ConnectionState::Idle => handle_connection_idle(&server_state, &mut state, &addr, &mut messages).await,
            ConnectionState::PublicMatchWaiting => todo!(),
            ConnectionState::PrivateMatchWaiting => todo!(),
            ConnectionState::Playing => todo!(),
        } {
            Ok(()) => {}
            Err(e) => {
                error!("{}", e);
                break;
            }
        }
        match messages.flush().await {
            Ok(()) => {}
            Err(e) => {
                error!("[{}:{}] {}", addr.ip(), addr.port(), e);
                break;
            }
        };
    }
}

async fn handle_connection_idle(
    server_state: &Arc<ServerState>,
    state: &mut ConnectionState,
    addr: &SocketAddr,
    messages: &mut Framed<TcpStream, MessageCodec>,
) -> Result<()> {
    let message_option = messages.next().await;
    match message_option {
        Some(Ok((message_type, _message_bytes))) => match message_type {
            MessageType::C2SGreet => {
                let mut response_bytes = BytesMut::new();
                response_bytes.reserve(8 * 6);
                let mut buffer = [0; 8];
                LittleEndian::write_i64(&mut buffer, 1);
                response_bytes.extend(buffer); // version, unconfirmed
                LittleEndian::write_i64(&mut buffer, 0);
                for _ in 0..5 {
                    // unknown zeros
                    response_bytes.extend(buffer);
                }
                match messages
                    .feed((MessageType::S2CGreet, response_bytes.into()))
                    .await
                {
                    Ok(()) => {}
                    Err(e) => {
                        error!("[{}:{}] {}", addr.ip(), addr.port(), e);
                        return Err(e);
                    }
                }
            }
            MessageType::C2SMatchCreateOrJoin => {
                //messages.feed().await;
                match Visibility::Public {
                    Visibility::Public => {
                        *state = ConnectionState::PublicMatchWaiting;
                    }
                    Visibility::Private => {
                        *state = ConnectionState::PrivateMatchWaiting;
                    }
                }
            }
            MessageType::C2SMatchCancel => {
                let mut response_bytes = BytesMut::new();
                response_bytes.reserve(8 * 1);
                let mut buffer = [0; 8];
                LittleEndian::write_i64(&mut buffer, 0);
                response_bytes.extend(buffer); // cancel count
                match messages
                    .feed((MessageType::S2CMatchCancelSuccess, response_bytes.into()))
                    .await
                {
                    Ok(()) => {}
                    Err(e) => {
                        error!("[{}:{}] {}", addr.ip(), addr.port(), e);
                        return Err(e);
                    }
                }
            }
            MessageType::C2SMatchListRequest => {
                //messages.feed().await;
            }
            _ => {
                error!(
                    "[{}:{}] Invalid message type {:?} at state Idle.",
                    addr.ip(),
                    addr.port(),
                    message_type
                );
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, ""));
            }
        },

        Some(Err(e)) => {
            error!("[{}:{}] {}", addr.ip(), addr.port(), e);
            return Err(e);
        }
        None => {}
    }
    Ok(())
}
