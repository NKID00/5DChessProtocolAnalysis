use anyhow::{Error, Result};
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::select;
use tokio::sync::{broadcast, watch, Mutex};
use tokio::time::{sleep, Instant, Sleep};
use tracing::{error, info, trace};
use serde::Deserialize;

use crate::{datatype::*, Config};

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub allow_reset_puzzle: bool,
    pub variants: HashSet<Variant>,
    pub variants_without_random: Vec<Variant>,
}

impl ServerConfig {

}

#[derive(Debug)]
pub struct ServerState {
    pub match_id: AtomicI64,
    pub matches: Mutex<HashMap<Passcode, broadcast::Receiver<Message>>>,
    pub public_matches: Mutex<HashMap<Passcode, MatchSettingsWithoutVisibility>>,
    pub server_history_matches: Mutex<IndexMap<MatchId, ServerHistoryMatch>>,
    pub start_timestamp: Instant,
    pub config: ServerConfig,
}

impl ServerState {
    pub fn new(config: Config) -> Self {
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
        let mut variants_without_random = variants.clone();
        variants_without_random.remove(&Variant::Random);
        ServerState {
            match_id: AtomicI64::new(1),
            matches: Mutex::new(HashMap::new()),
            public_matches: Mutex::new(HashMap::new()),
            server_history_matches: Mutex::new(IndexMap::new()),
            start_timestamp: Instant::now(),
            allow_reset_puzzle,
            variants,
            variants_without_random: Vec::from_iter(variants_without_random),
        }
    }
    // pub fn new(allow_reset_puzzle: bool, variants: HashSet<Variant>) -> Self {
    //     let mut variants_without_random = variants.clone();
    //     variants_without_random.remove(&Variant::Random);
    //     ServerState {
    //         match_id: AtomicI64::new(1),
    //         matches: Mutex::new(HashMap::new()),
    //         public_matches: Mutex::new(HashMap::new()),
    //         server_history_matches: Mutex::new(IndexMap::new()),
    //         start_timestamp: Instant::now(),
    //         allow_reset_puzzle,
    //         variants,
    //         variants_without_random: Vec::from_iter(variants_without_random),
    //     }
    // }
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
    pub addr: SocketAddr,                         // client
    pub io: MessageIO,                            // client
    pub tx: Option<broadcast::Sender<Message>>,   // peer
    pub rx: Option<broadcast::Receiver<Message>>, // peer
    pub m: Option<MatchSettings>,                 // match is reserved as a key word
    pub running: watch::Receiver<bool>,
    pub timeout: Sleep,
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
            timeout: sleep(ss.)
        }
    }
}

fn trace_error<E>(cs: &mut ConnectionState, e: E)
where
    E: Display,
{
    error!("[{}:{}] {}", cs.addr.ip(), cs.addr.port(), e);
}

pub async fn handle_connection(
    ss: Arc<ServerState>,
    stream: TcpStream,
    addr: SocketAddr,
    running: watch::Receiver<bool>,
) {
    info!("[{}:{}] Connected.", addr.ip(), addr.port());
    let mut cs = ConnectionState::new(ss, addr, stream, running);
    match handle_connection_main_loop(&mut cs).await {
        Ok(()) => {}
        Err(e) => match e.downcast::<std::io::Error>() {
            Ok(e) if e.kind() == ErrorKind::ConnectionAborted => {}
            Ok(e) => trace_error(&mut cs, e),
            Err(e) => match e.downcast::<broadcast::error::RecvError>() {
                Ok(broadcast::error::RecvError::Closed) => {}
                Ok(e) => trace_error(&mut cs, e),
                Err(e) => trace_error(&mut cs, e),
            },
        },
    };

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
                    v.state = HistoryMatchState::Completed;
                }
                None => {}
            }
        }
    }
    let _ = cs.io.close().await;
    info!("[{}:{}] Disconnected.", cs.addr.ip(), cs.addr.port());
}

async fn handle_connection_main_loop(cs: &mut ConnectionState) -> Result<()> {
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
                result = cs.rx.as_mut().unwrap().recv() => {
                    match result {
                        Ok(msg) => handle_connection_playing(cs, msg).await?,
                        Err(e) => {
                            if e == broadcast::error::RecvError::Closed {
                                // handle unexpected opponent disconnect
                                handle_connection_playing(cs, Message::InternalForfeit).await?;
                            } else {
                                Err(e)?
                            }
                        },
                    };
                },
                result = cs.running.changed() => break result?
            },
        }
        cs.io.flush().await?;
    }
    Ok(())
}

fn peer_send(cs: &mut ConnectionState, msg: Message) -> Result<()> {
    trace!("Internal {:?}", msg);
    cs.tx.as_mut().unwrap().send(msg)?;
    Ok(())
}

async fn handle_match_list_request(
    cs: &mut ConnectionState,
    m: Option<MatchSettings>,
) -> Result<()> {
    let mut public_matches_count = 0;
    let server_history_matches_count = cs.ss.server_history_matches.lock().await.len();
    let mut body = S2CMatchListNonhostBody {
        public_matches: [MatchSettingsWithoutVisibility {
            color: OptionalColorWithRandom::None,
            clock: OptionalClock::None,
            variant: Variant::Standard,
            passcode: 0,
            match_id: -1,
        }; 13],
        public_matches_count,
        server_history_matches: [S2CMatchListServerHistoryMatch {
            state: HistoryMatchState::Completed,
            clock: OptionalClock::None,
            variant: Variant::Standard,
            visibility: Visibility::Public,
            seconds_passed: 0,
        }; 13],
        server_history_matches_count,
    };
    for (_i, (_passcode, public_match)) in cs.ss.public_matches.lock().await.iter().enumerate() {
        match m {
            // skip host match
            Some(m) if m.match_id == public_match.match_id => {}
            _ => {
                if public_matches_count >= 13 {
                    break;
                }
                body.public_matches[public_matches_count] = public_match.clone();
                public_matches_count += 1;
            }
        }
    }
    body.public_matches_count = public_matches_count;
    for (i, (_match_id, server_history_match)) in
        cs.ss.server_history_matches.lock().await.iter().enumerate()
    {
        body.server_history_matches[server_history_matches_count - i - 1] =
            server_history_match.clone().into();
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

async fn handle_connection_idle(cs: &mut ConnectionState, msg: Message) -> Result<()> {
    match msg {
        Message::C2SGreet(_body) => {
            cs.io.put(Message::S2CGreet).await?;
        }
        Message::C2SMatchCreateOrJoin(C2SMatchCreateOrJoinBody::Create(mut m)) => {
            // create match
            if !cs.ss.variants.contains(&m.variant) {
                err_invalid_data!("Variant {:?} is not allowed.", m.variant)?;
            }
            m.passcode = generate_random_passcode_internal_with_exceptions(&cs.ss.matches).await;
            let (tx, rx_peer) = broadcast::channel(8);
            let (tx_peer, rx) = broadcast::channel(8);
            cs.tx = Some(tx);
            cs.rx = Some(rx);
            // store tx_peer in rx_peer
            peer_send(cs, Message::InternalInitialize(tx_peer))?;
            // add to match list
            cs.ss.matches.lock().await.insert(m.passcode, rx_peer);
            m.match_id = cs.ss.match_id.fetch_add(1, Ordering::Relaxed);
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
            let rx = cs.ss.matches.lock().await.remove(&passcode);
            match rx {
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
                        Message::InternalInitialize(tx) => tx,
                        _ => unreachable!(),
                    };
                    cs.tx = Some(tx);
                    // notify peer
                    peer_send(cs, Message::InternalJoin)?;
                    // receive match information from peer
                    let body = match rx.recv().await? {
                        Message::InternalMatchStart(body) => body,
                        _ => unreachable!(),
                    };
                    cs.rx = Some(rx);
                    let mut server_history_matches = cs.ss.server_history_matches.lock().await;
                    server_history_matches.insert(
                        body.match_id,
                        ServerHistoryMatch::new(MatchSettings::new(body.m, visibility)),
                    );
                    if server_history_matches.len() > 13 {
                        server_history_matches.shift_remove_index(0);
                    }
                    cs.m = Some(MatchSettings::new(body.m, visibility));
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
                            match_id: body.match_id,
                            seconds_passed: body.seconds_passed,
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
        Message::C2SForfeit => {}
        Message::C2SMatchListRequest => handle_match_list_request(cs, None).await?,
        other => err_invalid_data!("Invalid message {:?} at state Idle.", other)?,
    }
    Ok(())
}

async fn handle_connection_waiting(cs: &mut ConnectionState, msg: Message) -> Result<()> {
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
        Message::InternalJoin => {
            let mut body = S2CMatchStartBody {
                m: cs.m.unwrap().into(),
                match_id: cs.m.unwrap().match_id,
                seconds_passed: Instant::now().duration_since(cs.ss.start_timestamp).as_secs(),
            };
            cs.state = ConnectionStateEnum::Playing;
            body.m.variant = body.m.variant.determined(&cs.ss.variants_without_random);
            body.m.color = body.m.color.determined();
            cs.io.put(Message::S2CMatchStart(body)).await?;
            body.m.color = body.m.color.reversed();
            peer_send(cs, Message::InternalMatchStart(body))?;
        }
        other => err_invalid_data!("Invalid message {:?} at state Waiting.", other)?,
    }
    Ok(())
}

async fn handle_connection_playing(cs: &mut ConnectionState, msg: Message) -> Result<()> {
    match msg {
        Message::C2SForfeit => {
            peer_send(cs, Message::InternalForfeit)?;
            let match_id = cs.m.unwrap().match_id;
            let mut server_history_matches = cs.ss.server_history_matches.lock().await;
            match server_history_matches.get_mut(&match_id) {
                Some(v) => {
                    v.state = HistoryMatchState::Completed;
                }
                None => {}
            }
            cs.tx = None;
            cs.rx = None;
            cs.m = None;
            cs.state = ConnectionStateEnum::Idle;
        }
        Message::C2SOrS2CAction(mut body) => {
            if (!cs.ss.allow_reset_puzzle) && body.action_type == ActionType::ResetPuzzle {
                err_invalid_data!("Action of type {:?} is not allowed.", body.action_type)?;
            }
            body.seconds_passed = Instant::now().duration_since(cs.ss.start_timestamp).as_secs();
            peer_send(cs, Message::InternalAction(body))?;
            cs.io.put(Message::C2SOrS2CAction(body)).await?;
        }
        Message::C2SMatchListRequest => handle_match_list_request(cs, None).await?,
        Message::InternalForfeit => {
            let match_id = cs.m.unwrap().match_id;
            let mut server_history_matches = cs.ss.server_history_matches.lock().await;
            match server_history_matches.get_mut(&match_id) {
                Some(v) => {
                    v.state = HistoryMatchState::Completed;
                }
                None => {}
            }
            cs.tx = None;
            cs.rx = None;
            cs.m = None;
            cs.state = ConnectionStateEnum::Idle;
            cs.io.put(Message::S2COpponentLeft).await?;
        }
        Message::InternalAction(body) => {
            cs.io.put(Message::C2SOrS2CAction(body)).await?;
        }
        other => err_invalid_data!("Invalid message {:?} at state Playing.", other)?,
    }
    Ok(())
}
