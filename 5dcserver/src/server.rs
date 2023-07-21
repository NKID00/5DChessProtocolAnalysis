use anyhow::{Error, Result};
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{broadcast, watch, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Instant, Sleep};
use tokio::{select, spawn};
use tracing::{error, info, trace};

use crate::{datatype::*, Config};

#[macro_export]
macro_rules! err_timeout {
    () => {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Timed out.",
        ))
    };
}
#[macro_export]
macro_rules! err_limit {
    ( $($arg:tt)* ) => {
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!($($arg)*),
        ))
    };
}

#[derive(Debug)]
pub struct ServerConfig {
    pub ban_public_match: bool,
    pub ban_private_match: bool,
    pub ban_reset_puzzle: bool,
    pub variants: HashSet<Variant>,
    pub variants_without_random: HashSet<Variant>,

    pub limit_concurrent_match: usize,
    pub limit_public_waiting: usize,
    pub limit_connection_duration: Duration,
    pub limit_message_length: usize,
}

impl ServerConfig {
    fn new(config: Config) -> Result<Self> {
        let mut variants = HashSet::new();
        for i in 1..46 {
            variants.insert(try_i64_to_enum(i)?);
        }
        for i in config.ban_variant {
            variants.remove(&try_i64_to_enum(i)?);
        }
        let mut variants_without_random = variants.clone();
        variants_without_random.remove(&Variant::Random);
        Ok(ServerConfig {
            ban_public_match: config.ban_public_match,
            ban_private_match: config.ban_private_match,
            ban_reset_puzzle: config.ban_reset_puzzle,
            variants,
            variants_without_random,
            limit_concurrent_match: config.limit_concurrent_match,
            limit_public_waiting: config.limit_public_waiting,
            limit_connection_duration: Duration::from_secs(config.limit_connection_duration),
            limit_message_length: config.limit_message_length,
        })
    }
}

#[derive(Debug)]
pub struct ServerState {
    pub match_id: AtomicI64,
    pub matches: RwLock<HashMap<Passcode, broadcast::Receiver<Message>>>,
    pub public_matches: RwLock<HashMap<Passcode, MatchSettingsWithoutVisibility>>,
    pub server_history_matches: RwLock<IndexMap<MatchId, ServerHistoryMatch>>,
    pub start_timestamp: Instant,
    pub config: ServerConfig,
    pub running: watch::Receiver<bool>,
}

impl ServerState {
    pub fn new(config: Config, running: watch::Receiver<bool>) -> Result<Self> {
        Ok(ServerState {
            match_id: AtomicI64::new(1),
            matches: RwLock::new(HashMap::new()),
            public_matches: RwLock::new(HashMap::new()),
            server_history_matches: RwLock::new(IndexMap::new()),
            start_timestamp: Instant::now(),
            config: ServerConfig::new(config)?,
            running,
        })
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
    pub addr: SocketAddr,                         // client
    pub io: MessageIO,                            // client
    pub tx: Option<broadcast::Sender<Message>>,   // peer
    pub rx: Option<broadcast::Receiver<Message>>, // peer
    pub m: Option<MatchSettings>,                 // match is reserved as a key word
    pub running: watch::Receiver<bool>,
    pub timeout: JoinHandle<()>,
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
            io: MessageIO::new(stream, ss.config.limit_message_length),
            tx: None,
            rx: None,
            m: None,
            running,
            timeout: spawn(sleep(ss.config.limit_connection_duration)),
        }
    }
}

fn trace_error<E>(cs: &mut ConnectionState, e: E)
where
    E: Display,
{
    error!("[{}:{}] {}", cs.addr.ip(), cs.addr.port(), e);
}

pub async fn handle_connection(ss: Arc<ServerState>, stream: TcpStream, addr: SocketAddr) {
    info!("[{}:{}] Connected.", addr.ip(), addr.port());
    let running = ss.running.clone();
    let mut cs = ConnectionState::new(ss, addr, stream, running);
    if let Err(e) = handle_connection_main_loop(&mut cs).await {
        match e.downcast::<std::io::Error>() {
            Ok(e) if e.kind() == ErrorKind::ConnectionAborted => {}
            Ok(e) => trace_error(&mut cs, e),
            Err(e) => match e.downcast::<broadcast::error::RecvError>() {
                Ok(broadcast::error::RecvError::Closed) => {}
                Ok(e) => trace_error(&mut cs, e),
                Err(e) => trace_error(&mut cs, e),
            },
        }
    };

    // clean resources, remove match from public match list, etc.
    match cs.state {
        ConnectionStateEnum::Idle => {}
        ConnectionStateEnum::Waiting => {
            let m = cs.m.unwrap();
            if m.visibility == Visibility::Public {
                cs.ss.public_matches.write().await.remove(&m.passcode);
            }
            cs.ss.matches.write().await.remove(&m.passcode);
        }
        ConnectionStateEnum::Playing => {
            let match_id = cs.m.unwrap().match_id;
            let mut server_history_matches = cs.ss.server_history_matches.write().await;
            if let Some(v) = server_history_matches.get_mut(&match_id) {
                v.state = HistoryMatchState::Completed;
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
                result = cs.running.changed() => break result?,
                _ = &mut cs.timeout => break,
            },
            ConnectionStateEnum::Waiting => select! {
                result = cs.io.get() => handle_connection_waiting(cs, result?).await?,
                result = cs.rx.as_mut().unwrap().recv() => handle_connection_waiting(cs, result?).await?,
                result = cs.running.changed() => break result?,
                _ = &mut cs.timeout => break,
            },
            ConnectionStateEnum::Playing => select! {
                result = cs.io.get() => handle_connection_playing(cs, result?).await?,
                result = cs.rx.as_mut().unwrap().recv() => match result {
                    Ok(msg) => handle_connection_playing(cs, msg).await?,
                    Err(e) if e == broadcast::error::RecvError::Closed => {
                        // handle unexpected opponent disconnect
                        handle_connection_playing(cs, Message::InternalForfeit).await?;
                    }
                    Err(e) => Err(e)?
                },
                result = cs.running.changed() => break result?,
                _ = &mut cs.timeout => break,
            },
        }
        cs.io.flush().await?;
    }
    Ok(())
}

fn send_to_peer(cs: &mut ConnectionState, msg: Message) -> Result<()> {
    trace!("Internal {:?}", msg);
    cs.tx.as_mut().unwrap().send(msg)?;
    Ok(())
}

async fn handle_match_list_request(
    cs: &mut ConnectionState,
    m: Option<MatchSettings>,
) -> Result<()> {
    let mut public_matches_count = 0;
    let server_history_matches_count = cs.ss.server_history_matches.read().await.len();
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
    for (_i, (_passcode, public_match)) in cs.ss.public_matches.read().await.iter().enumerate() {
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
        cs.ss.server_history_matches.read().await.iter().enumerate()
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
            if !cs.ss.config.variants.contains(&m.variant) {
                err_invalid_data!("Variant {:?} is not allowed.", m.variant)?;
            }
            if cs.ss.matches.read().await.len() >= cs.ss.config.limit_concurrent_match {
                err_limit!("Concurrent matches limit exceeded.")?;
            }
            let mut match_list = cs.ss.public_matches.write().await;
            if m.visibility == Visibility::Public
                && match_list.len() >= cs.ss.config.limit_public_waiting
            {
                err_limit!("Public waiting matches limit exceeded.")?;
            }
            m.passcode = generate_random_passcode_internal_with_exceptions(&cs.ss.matches).await;
            let (tx, rx_peer) = broadcast::channel(16);
            let (tx_peer, rx) = broadcast::channel(16);
            cs.tx = Some(tx);
            cs.rx = Some(rx);
            // store tx_peer in rx_peer
            send_to_peer(cs, Message::InternalInitialize(tx_peer))?;
            // insert into match list
            cs.ss.matches.write().await.insert(m.passcode, rx_peer);
            match m.visibility {
                Visibility::Public => {
                    m.match_id = cs.ss.match_id.fetch_add(1, Ordering::Relaxed);
                    // insert into public match list
                    match_list.insert(m.passcode, m.clone().into());
                }
                Visibility::Private => {
                    m.match_id = cs.ss.match_id.fetch_add(1, Ordering::Relaxed);
                }
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
            let rx = cs.ss.matches.write().await.remove(&passcode);
            match rx {
                Some(mut rx) => {
                    // match found
                    // remove from public match list
                    let visibility = if cs
                        .ss
                        .public_matches
                        .write()
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
                    send_to_peer(cs, Message::InternalJoin)?;
                    // receive match information from peer
                    let body = match rx.recv().await? {
                        Message::InternalMatchStart(body) => body,
                        _ => unreachable!(),
                    };
                    cs.rx = Some(rx);
                    let mut server_history_matches = cs.ss.server_history_matches.write().await;
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
                seconds_passed: Instant::now()
                    .duration_since(cs.ss.start_timestamp)
                    .as_secs(),
            };
            cs.state = ConnectionStateEnum::Playing;
            body.m.variant = body.m.variant.determined(&cs.ss.variants_without_random);
            body.m.color = body.m.color.determined();
            cs.io.put(Message::S2CMatchStart(body)).await?;
            body.m.color = body.m.color.reversed();
            send_to_peer(cs, Message::InternalMatchStart(body))?;
        }
        other => err_invalid_data!("Invalid message {:?} at state Waiting.", other)?,
    }
    Ok(())
}

async fn handle_connection_playing(cs: &mut ConnectionState, msg: Message) -> Result<()> {
    match msg {
        Message::C2SForfeit => {
            send_to_peer(cs, Message::InternalForfeit)?;
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
            body.seconds_passed = Instant::now()
                .duration_since(cs.ss.start_timestamp)
                .as_secs();
            send_to_peer(cs, Message::InternalAction(body))?;
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
