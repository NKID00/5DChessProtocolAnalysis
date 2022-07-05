use indexmap::IndexMap;
use std::collections::HashMap;
use std::io::Result;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::{net::TcpStream, select};
use tracing::{error, info};

use crate::datatype::*;

#[derive(Debug)]
pub struct ServerState {
    pub match_id: Mutex<u64>,
    pub message_id: Mutex<u64>,
    pub matches: Mutex<HashMap<Passcode, MatchSettings>>,
    pub public_matches: Mutex<HashMap<Passcode, MatchSettingsWithoutVisibility>>,
    pub server_history_matches: Mutex<IndexMap<i64 /* Match ID */, ServerHistoryMatch>>,
}

impl ServerState {
    pub fn new() -> Self {
        ServerState {
            match_id: Mutex::new(0),
            message_id: Mutex::new(0),
            matches: Mutex::new(HashMap::<Passcode, MatchSettings>::new()),
            public_matches: Mutex::new(HashMap::<Passcode, MatchSettingsWithoutVisibility>::new()),
            server_history_matches: Mutex::new(IndexMap::<i64, ServerHistoryMatch>::new()),
        }
    }
}

/* state machine of one connection:
Idle -> PublicWaiting -> Playing -> Idle
Idle -> PrivateWaiting -> Playing -> Idle
*/
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ConnectionStateEnum {
    Idle,
    Waiting,
    Playing,
}

#[derive(Debug)]
pub struct ConnectionState {
    pub state: ConnectionStateEnum,
    pub ss: Arc<ServerState>,
    pub addr: SocketAddr,
    pub addr_peer: Option<SocketAddr>,
    pub io: MessageIO,
    pub io_peer: Option<MessageIO>,
    pub m: Option<MatchSettings>, // match is reserved as a key word
}

impl ConnectionState {
    pub fn new(ss: Arc<ServerState>, addr: SocketAddr, stream: TcpStream) -> Self {
        ConnectionState {
            state: ConnectionStateEnum::Idle,
            ss,
            addr,
            addr_peer: None,
            io: MessageIO::new(stream),
            io_peer: None,
            m: None,
        }
    }

    pub fn waiting(&mut self, m: MatchSettings) {
        self.state = ConnectionStateEnum::Waiting;
        self.m = Some(m);
    }

    pub fn playing(&mut self, cs_peer: ConnectionState) {
        self.state = ConnectionStateEnum::Playing;
        self.addr_peer = Some(cs_peer.addr);
        self.io_peer = Some(cs_peer.io);
    }
}

pub async fn handle_connection(ss: Arc<ServerState>, stream: TcpStream, addr: SocketAddr) {
    let mut cs = ConnectionState::new(ss, addr, stream);
    match handle_connection_main_loop(&mut cs).await {
        Ok(()) => {}
        Err(e) => match cs.state {
            ConnectionStateEnum::Playing => match cs.addr_peer {
                Some(ref addr_peer) => {
                    error!(
                        "[{}:{}, {}:{}] {}",
                        cs.addr.ip(),
                        cs.addr.port(),
                        addr_peer.ip(),
                        addr_peer.port(),
                        e
                    );
                }
                None => unreachable!(),
            },
            _ => {
                error!("[{}:{}] {}", cs.addr.ip(), cs.addr.port(), e);
            }
        },
    }
    // clean resources, remove match from public_matches, etc.
    match cs.state {
        ConnectionStateEnum::Idle => {
            info!("[{}:{}] Disconnected.", addr.ip(), addr.port());
        }
        ConnectionStateEnum::Waiting => {
            match cs.m {
                Some(m) => {
                    if m.visibility == Visibility::Public {
                        cs.ss.public_matches.lock().await.remove(&m.passcode);
                    }
                    cs.ss.matches.lock().await.remove(&m.passcode);
                }
                None => unreachable!(),
            }
            error!("[{}:{}] Disconnected", cs.addr.ip(), cs.addr.port());
        }
        ConnectionStateEnum::Playing => {
            let match_id = match cs.m {
                Some(m) => m.match_id,
                None => unreachable!(),
            };
            {
                let mut server_history_matches = cs.ss.server_history_matches.lock().await;
                match server_history_matches.get_mut(&match_id) {
                    Some(v) => {
                        v.status = HistoryMatchStatus::Completed;
                    }
                    None => {}
                }
            }
            match cs.addr_peer {
                Some(ref addr_peer) => {
                    info!(
                        "[{}:{}, {}:{}] Disconnected",
                        cs.addr.ip(),
                        cs.addr.port(),
                        addr_peer.ip(),
                        addr_peer.port()
                    );
                }
                None => unreachable!(),
            }
        }
    }
}

async fn handle_connection_main_loop(cs: &mut ConnectionState) -> Result<()> {
    loop {
        match cs.state {
            ConnectionStateEnum::Idle => match cs.io.get().await? {
                Message::C2SGreet(_body) => {
                    cs.io.put(Message::S2CGreet).await?;
                }
                Message::C2SMatchCreateOrJoin(C2SMatchCreateOrJoinBody::Create(m)) => {
                    // create match
                    let passcode =
                        generate_random_passcode_internal_with_exceptions(&cs.ss.matches).await;
                    cs.ss.matches.lock().await.insert(passcode, m.clone());
                    if m.visibility == Visibility::Public {
                        // add to public match list
                        cs.ss
                            .public_matches
                            .lock()
                            .await
                            .insert(m.passcode, m.clone().into());
                        // TODO: public match max count
                    }
                    cs.waiting(m);
                }
                Message::C2SMatchCreateOrJoin(C2SMatchCreateOrJoinBody::Join(passcode)) => {
                    // join match
                    match cs.ss.matches.lock().await.get(&passcode) {
                        Some(m) => {
                            // match found
                            if m.visibility == Visibility::Public {
                                cs.ss.public_matches.lock().await.remove(&passcode);
                            }
                            cs.io
                                .put(Message::S2CMatchCreateOrJoinResult(
                                    S2CMatchCreateOrJoinResultBody::Success(m.clone()),
                                ))
                                .await?;
                        }
                        None => {
                            // match not found
                            cs.io
                                .put(Message::S2CMatchCreateOrJoinResult(
                                    S2CMatchCreateOrJoinResultBody::Failed,
                                ))
                                .await?;
                        }
                    }
                }
                Message::C2SMatchCancel => {
                    cs.io
                        .put(Message::S2CMatchCancelResult(
                            S2CMatchCancelResultBody::Failed,
                        ))
                        .await?;
                }
                Message::C2SMatchListRequest => {
                    let mut public_matches_count = cs.ss.public_matches.lock().await.len();
                    if public_matches_count > 13 {
                        public_matches_count = 13;
                    }
                    let server_history_matches_count =
                        cs.ss.server_history_matches.lock().await.len();
                    let mut body = S2CMatchListNonhostBody {
                        public_matches: [MatchSettingsWithoutVisibility {
                            color: OptionalColorWithRandom::None,
                            clock: OptionalClock::None,
                            variant: 0,
                            passcode: 0,
                            match_id: -1,
                        }; 13],
                        public_matches_count,
                        server_history_matches: [S2CMatchListServerHistoryMatch {
                            status: HistoryMatchStatus::Completed,
                            clock: OptionalClock::None,
                            variant: 0,
                            visibility: Visibility::Public,
                            seconds_passed: 0,
                        }; 13],
                        server_history_matches_count,
                    };
                    for (i, (_, m)) in cs.ss.public_matches.lock().await.iter().enumerate() {
                        if i >= 13 {
                            break;
                        }
                        body.public_matches[i] = m.clone();
                    }
                    for (i, (_, m)) in cs.ss.server_history_matches.lock().await.iter().enumerate()
                    {
                        body.server_history_matches[i] = m.clone().into();
                    }
                    cs.io
                        .put(Message::S2CMatchList(S2CMatchListBody::Nonhost(body)))
                        .await?;
                }
                other => {
                    return err_invalid_data!(
                        "Invalid message type {:?} at state Idle.",
                        other.message_type()
                    );
                }
            },

            ConnectionStateEnum::Waiting => {
                let message = cs.io.get().await?;
                todo!()
            }

            ConnectionStateEnum::Playing => select! {
                result = cs.io.get() => {
                    let message = result?;
                    todo!()
                },
                result = match cs.io_peer {
                    Some(ref mut io) => io.get(),
                    None => unreachable!(),
                } => {
                    let message = result?;
                    todo!()
                }
            },
        }
    }
}
