use indexmap::IndexMap;
use std::collections::HashMap;
use std::error::Error;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::{broadcast, watch, Mutex};
use tracing::{error, info};

use crate::datatype::*;

#[derive(Debug)]
pub struct ServerState {
    pub match_id: AtomicI64,
    pub message_id: AtomicI64,
    pub matches: Mutex<HashMap<Passcode, broadcast::Receiver<Message>>>,
    pub public_matches: Mutex<HashMap<Passcode, MatchSettingsWithoutVisibility>>,
    pub server_history_matches: Mutex<IndexMap<MatchId, ServerHistoryMatch>>,
}

impl ServerState {
    pub fn new() -> Self {
        ServerState {
            match_id: AtomicI64::new(1),
            message_id: AtomicI64::new(1),
            matches: Mutex::new(HashMap::new()),
            public_matches: Mutex::new(HashMap::new()),
            server_history_matches: Mutex::new(IndexMap::new()),
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
    pub io: MessageIO,
    pub tx: Option<broadcast::Sender<Message>>,
    pub rx: Option<broadcast::Receiver<Message>>,
    pub m: Option<MatchSettings>, // match is reserved as a key word
    pub running: watch::Receiver<bool>,
}

impl ConnectionState {
    pub fn new(
        ss: Arc<ServerState>,
        addr: SocketAddr,
        stream: TcpStream,
        running: watch::Receiver<bool>,
    ) -> Self {
        ConnectionState {
            state: ConnectionStateEnum::Idle,
            ss,
            addr,
            io: MessageIO::new(stream),
            tx: None,
            rx: None,
            m: None,
            running,
        }
    }
}

fn trace_error(cs: &mut ConnectionState, e: Box<dyn Error>) {
    error!("[{}:{}] {}", cs.addr.ip(), cs.addr.port(), e);
}

pub async fn handle_connection(
    ss: Arc<ServerState>,
    stream: TcpStream,
    addr: SocketAddr,
    running: watch::Receiver<bool>,
) {
    let mut cs = ConnectionState::new(ss, addr, stream, running);
    match handle_connection_main_loop(&mut cs).await {
        Ok(()) => {}
        Err(e) => match e.downcast::<std::io::Error>() {
            Ok(e) if e.kind() == ErrorKind::ConnectionAborted => {}
            Ok(e) => trace_error(&mut cs, e),
            Err(e) => trace_error(&mut cs, e),
        },
    }

    // clean resources, remove match from public match list, etc.
    match cs.state {
        ConnectionStateEnum::Idle => {}
        ConnectionStateEnum::Waiting => {
            let m = cs.m.unwrap();
            if m.visibility == Visibility::Public {
                cs.ss.public_matches.lock().await.remove(&m.passcode);
            }
            cs.ss.matches.lock().await.remove(&m.passcode);
        }
        ConnectionStateEnum::Playing => {
            let match_id = cs.m.unwrap().match_id;
            let mut server_history_matches = cs.ss.server_history_matches.lock().await;
            match server_history_matches.get_mut(&match_id) {
                Some(v) => {
                    v.status = HistoryMatchStatus::Completed;
                }
                None => {}
            }
        }
    }
    let _ = cs.io.close().await;
    info!("[{}:{}] Disconnected", cs.addr.ip(), cs.addr.port());
}

async fn handle_connection_main_loop(cs: &mut ConnectionState) -> Result<(), Box<dyn Error>> {
    loop {
        match cs.state {
            ConnectionStateEnum::Idle => select! {
                result = cs.io.get() => handle_connection_idle(cs, result?).await?,
                result = cs.running.changed() => break result?
            },
            ConnectionStateEnum::Waiting => select! {
                result = cs.io.get() => handle_connection_waiting(cs, result?).await?,
                result = cs.rx.as_mut().unwrap().recv() => handle_connection_waiting(cs, result?).await?,
                result = cs.running.changed() => break result?
            },
            ConnectionStateEnum::Playing => select! {
                result = cs.io.get() => handle_connection_playing(cs, result?).await?,
                result = cs.rx.as_mut().unwrap().recv() => handle_connection_playing(cs, result?).await?,
                result = cs.running.changed() => break result?
            },
        }
        cs.io.flush().await?;
    }
    Ok(())
}

fn peer_send(cs: &mut ConnectionState, msg: Message) -> Result<(), Box<dyn Error>> {
    cs.tx.as_mut().unwrap().send(msg)?;
    Ok(())
}

async fn handle_match_list_request(
    cs: &mut ConnectionState,
    m: Option<MatchSettings>,
) -> Result<(), Box<dyn Error>> {
    let mut public_matches_count = cs.ss.public_matches.lock().await.len();
    if public_matches_count > 13 {
        public_matches_count = 13;
    }
    let server_history_matches_count = cs.ss.server_history_matches.lock().await.len();
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
    for (i, (_, m)) in cs.ss.server_history_matches.lock().await.iter().enumerate() {
        body.server_history_matches[server_history_matches_count - i - 1] = m.clone().into();
    }
    match m {
        Some(m) => {
            cs.io
                .put(Message::S2CMatchList(S2CMatchListBody::Host(
                    S2CMatchListHostBody {
                        color: m.color,
                        clock: m.clock,
                        variant: m.variant,
                        passcode: m.passcode,
                        body,
                    },
                )))
                .await?;
        }
        None => {
            cs.io
                .put(Message::S2CMatchList(S2CMatchListBody::Nonhost(body)))
                .await?;
        }
    }
    Ok(())
}

async fn handle_connection_idle(
    cs: &mut ConnectionState,
    msg: Message,
) -> Result<(), Box<dyn Error>> {
    match msg {
        Message::C2SGreet(_body) => {
            cs.io.put(Message::S2CGreet).await?;
        }
        Message::C2SMatchCreateOrJoin(C2SMatchCreateOrJoinBody::Create(mut m)) => {
            // create match
            m.passcode = generate_random_passcode_internal_with_exceptions(&cs.ss.matches).await;
            let (tx, rx_peer) = broadcast::channel(8);
            let (tx_peer, rx) = broadcast::channel(8);
            // store tx_peer in rx_peer
            tx.send(Message::S2SInitialize(tx_peer))?;
            cs.tx = Some(tx);
            cs.rx = Some(rx);
            // add to match list
            cs.ss.matches.lock().await.insert(m.passcode, rx_peer);
            if m.visibility == Visibility::Public {
                // add to public match list
                cs.ss
                    .public_matches
                    .lock()
                    .await
                    .insert(m.passcode, m.clone().into());
                // TODO: limit number of public matches
            }
            cs.m = Some(m);
            cs.state = ConnectionStateEnum::Waiting;
            cs.io
                .put(Message::S2CMatchCreateOrJoinResult(
                    S2CMatchCreateOrJoinResultBody::Success(m.clone()),
                ))
                .await?;
        }
        Message::C2SMatchCreateOrJoin(C2SMatchCreateOrJoinBody::Join(passcode)) => {
            // join match
            // remove from match list
            match cs.ss.matches.lock().await.remove(&passcode) {
                Some(mut rx) => {
                    // match found
                    // remove from public match list
                    let visibility = if cs
                        .ss
                        .public_matches
                        .lock()
                        .await
                        .remove(&passcode)
                        .is_some()
                    {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };
                    // receive sender from peer
                    let tx = match rx.recv().await? {
                        Message::S2SInitialize(tx) => tx,
                        _ => unreachable!(),
                    };
                    // notify peer
                    tx.send(Message::S2SJoin)?;
                    // receive match information from peer
                    let body = match rx.recv().await? {
                        Message::S2SMatchStart(body) => body,
                        _ => unreachable!(),
                    };
                    let mut server_history_matches = cs.ss.server_history_matches.lock().await;
                    server_history_matches.insert(
                        body.match_id,
                        ServerHistoryMatch::new(MatchSettings::new(body.m, visibility)),
                    );
                    if server_history_matches.len() > 13 {
                        server_history_matches.shift_remove_index(0);
                    }
                    cs.tx = Some(tx);
                    cs.rx = Some(rx);
                    cs.state = ConnectionStateEnum::Playing;
                    cs.io
                        .put(Message::S2CMatchCreateOrJoinResult(
                            S2CMatchCreateOrJoinResultBody::Success(MatchSettings::new(
                                body.m, visibility,
                            )),
                        ))
                        .await?;
                    cs.io
                        .put(Message::S2CMatchStart(S2CMatchStartBody {
                            m: body.m,
                            match_id: cs.ss.match_id.fetch_add(1, Ordering::Relaxed),
                            message_id: cs.ss.message_id.fetch_add(1, Ordering::Relaxed),
                        }))
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
        Message::C2SMatchListRequest => handle_match_list_request(cs, None).await?,
        other => err_invalid_data!(
            "Invalid message of type {:?} at state Idle.",
            other.message_type()
        )?,
    }
    Ok(())
}

async fn handle_connection_waiting(
    cs: &mut ConnectionState,
    msg: Message,
) -> Result<(), Box<dyn Error>> {
    match msg {
        Message::C2SMatchCancel => {
            let passcode = cs.m.unwrap().passcode;
            cs.ss.public_matches.lock().await.remove(&passcode);
            cs.ss.matches.lock().await.remove(&passcode);
            cs.tx = None;
            cs.rx = None;
            cs.m = None;
            cs.state = ConnectionStateEnum::Idle;
            cs.io
                .put(Message::S2CMatchCancelResult(
                    S2CMatchCancelResultBody::Success,
                ))
                .await?;
        }
        Message::C2SMatchListRequest => handle_match_list_request(cs, cs.m).await?,
        Message::S2SJoin => {
            let mut body = S2CMatchStartBody {
                m: cs.m.unwrap().into(),
                match_id: cs.ss.match_id.fetch_add(1, Ordering::Relaxed),
                message_id: cs.ss.message_id.fetch_add(1, Ordering::Relaxed),
            };
            cs.state = ConnectionStateEnum::Playing;
            body.m.color = body.m.color.determined();
            cs.tx.as_mut().unwrap().send(Message::S2SMatchStart(body))?;
            body.m.color = body.m.color.reversed();
            cs.io.put(Message::S2CMatchStart(body)).await?;
        }
        other => err_invalid_data!(
            "Invalid message of type {:?} at state Waiting.",
            other.message_type()
        )?,
    }
    Ok(())
}

async fn handle_connection_playing(
    cs: &mut ConnectionState,
    msg: Message,
) -> Result<(), Box<dyn Error>> {
    match msg {
        Message::C2SForfeit => {
            peer_send(cs, Message::S2SForfeit)?;
            let match_id = cs.m.unwrap().match_id;
            let mut server_history_matches = cs.ss.server_history_matches.lock().await;
            match server_history_matches.get_mut(&match_id) {
                Some(v) => {
                    v.status = HistoryMatchStatus::Completed;
                }
                None => {}
            }
            cs.tx = None;
            cs.rx = None;
            cs.m = None;
            cs.state = ConnectionStateEnum::Idle;
        }
        Message::C2SOrS2CAction(body) => {
            peer_send(cs, Message::S2SAction(body))?;
            cs.io.put(Message::C2SOrS2CAction(body)).await?;
        }
        Message::S2SForfeit => {
            let match_id = cs.m.unwrap().match_id;
            let mut server_history_matches = cs.ss.server_history_matches.lock().await;
            match server_history_matches.get_mut(&match_id) {
                Some(v) => {
                    v.status = HistoryMatchStatus::Completed;
                }
                None => {}
            }
            cs.tx = None;
            cs.rx = None;
            cs.m = None;
            cs.state = ConnectionStateEnum::Idle;
            cs.io.put(Message::S2COpponentLeft).await?;
        }
        other => err_invalid_data!(
            "Invalid message of type {:?} at state Playing.",
            other.message_type()
        )?,
    }
    Ok(())
}
