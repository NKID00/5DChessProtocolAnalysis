use byteorder::{ByteOrder, LittleEndian};
use bytes::{Bytes, BytesMut};
use enum_primitive::{enum_from_primitive, enum_from_primitive_impl, enum_from_primitive_impl_ty};
use futures::{SinkExt, StreamExt};
use rand::Rng;
use std::collections::HashMap;
use std::io::Result;
use tokio::{net::TcpStream, sync::Mutex};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

pub const MESSAGE_LENGTH_MAX: usize = 4096; // >= 1008, prevent attacks

#[macro_export]
macro_rules! err_invalid_data {
    ( $($arg:tt)* ) => {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!($($arg)*),
        ))
    };
}

#[macro_export]
macro_rules! err_disconnected {
    () => {
        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionAborted,
            "Disconnected.",
        ))
    };
}

enum_from_primitive! {
    #[repr(i64)]
    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum OptionalColorWithRandom {
        None = 0,
        Random = 1,
        White = 2,
        Black = 3
    }
}

impl OptionalColorWithRandom {
    pub fn reversed(&self) -> Self {
        match self {
            OptionalColorWithRandom::White => OptionalColorWithRandom::Black,
            OptionalColorWithRandom::Black => OptionalColorWithRandom::White,
            _ => self.clone(),
        }
    }

    pub fn determined(&self) -> Self {
        match self {
            OptionalColorWithRandom::Random => match rand::thread_rng().gen_range(0..=1) {
                0 => OptionalColorWithRandom::White,
                1 => OptionalColorWithRandom::Black,
                _ => unreachable!(),
            },
            _ => self.clone(),
        }
    }
}

impl From<Color> for OptionalColorWithRandom {
    fn from(value: Color) -> Self {
        match value {
            Color::White => OptionalColorWithRandom::White,
            Color::Black => OptionalColorWithRandom::Black,
        }
    }
}

enum_from_primitive! {
    #[repr(i64)]
    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum Color {
        White = 0,
        Black = 1
    }
}

impl Color {
    pub fn reversed(&self) -> Self {
        match self {
            Color::White => Color::White,
            Color::Black => Color::Black,
        }
    }
}

impl TryFrom<OptionalColorWithRandom> for Color {
    type Error = std::io::Error;

    fn try_from(value: OptionalColorWithRandom) -> Result<Self> {
        match value {
            OptionalColorWithRandom::White => Ok(Color::White),
            OptionalColorWithRandom::Black => Ok(Color::Black),
            _ => err_invalid_data!("{:?} cannot be converted to Color.", value),
        }
    }
}

enum_from_primitive! {
    #[repr(i64)]
    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum OptionalClock {
        None = 0,
        NoClock = 1,
        Short = 2,
        Medium = 3,
        Long = 4
    }
}

enum_from_primitive! {
    #[repr(i64)]
    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum Visibility {
        Public = 1,
        Private = 2
    }
}

enum_from_primitive! {
    #[repr(i64)]
    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum ActionType {
        Move = 1,
        UndoMove = 2,
        SubmitMoves = 3,

        Header = 6
    }
}

enum_from_primitive! {
    #[repr(i64)]
    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum HistoryMatchStatus {
        Completed = 0,
        InProgress = 1
    }
}

#[derive(Debug, Copy, Clone)]
pub struct InternalMatch {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: i64,
    pub visibility: Visibility,
    pub passcode: i64,
}

#[derive(Debug, Copy, Clone)]
pub struct PublicMatch {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: i64,
    pub passcode: i64,
}

#[derive(Debug, Copy, Clone)]
pub struct PrivateMatch {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: i64,
    pub passcode: i64,
}

#[derive(Debug, Copy, Clone)]
pub struct ServerHistoryMatch {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: i64,
    pub visibility: Visibility,
    pub seconds_passed: u64,
}

enum_from_primitive! {
    #[repr(i64)]
    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum MessageType {
        C2SGreet = 1,
        S2CGreet = 2,
        C2SMatchCreateOrJoin = 3,
        S2CMatchCreateOrJoinResult = 4,
        C2SMatchCancel = 5,
        S2CMatchCancelResult = 6,
        S2CMatchStart = 7,

        S2COpponentLeft = 9,
        C2SForfeit = 10,
        C2SOrS2CAction = 11,
        C2SMatchListRequest = 12,
        S2CMatchList = 13
    }
}

impl MessageType {
    pub fn legal_length(&self) -> usize {
        match self {
            MessageType::C2SGreet => 56,
            MessageType::S2CGreet => 56,
            MessageType::C2SMatchCreateOrJoin => 48,
            MessageType::S2CMatchCreateOrJoinResult => 64,
            MessageType::C2SMatchCancel => 9,
            MessageType::S2CMatchCancelResult => 16,
            MessageType::S2CMatchStart => 48,
            MessageType::S2COpponentLeft => 9,
            MessageType::C2SForfeit => 9,
            MessageType::C2SOrS2CAction => 112,
            MessageType::C2SMatchListRequest => 9,
            MessageType::S2CMatchList => 1008,
        }
    }
}

// unknown or unused fields omitted
#[derive(Debug, Copy, Clone)]
pub enum Message {
    C2SGreet(C2SGreetBody),
    S2CGreet,
    C2SMatchCreateOrJoin(C2SMatchCreateOrJoinBody),
    S2CMatchCreateOrJoinResult(S2CMatchCreateOrJoinResultBody),
    C2SMatchCancel,
    S2CMatchCancelResult(S2CMatchCancelResultBody),
    S2CMatchStart(S2CMatchStartBody),
    S2COpponentLeft,
    C2SForfeit,
    C2SOrS2CAction(C2SOrS2CActionBody),
    C2SMatchListRequest,
    S2CMatchList(S2CMatchListBody),
}
#[derive(Debug, Copy, Clone)]
pub struct C2SGreetBody {
    pub version1: i64,
    pub version2: i64,
}
#[derive(Debug, Copy, Clone)]
pub struct C2SMatchCreateOrJoinBody {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: i64,
    pub visibility: Visibility,
    pub passcode: i64,
}
#[derive(Debug, Copy, Clone)]
pub enum S2CMatchCreateOrJoinResultBody {
    Success(S2CMatchCreateOrJoinResultSuccessBody),
    Failed,
}
#[derive(Debug, Copy, Clone)]
pub struct S2CMatchCreateOrJoinResultSuccessBody {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: i64,
    pub visibility: Visibility,
    pub passcode: i64,
}
#[derive(Debug, Copy, Clone)]
pub enum S2CMatchCancelResultBody {
    Success,
    Failed,
}
#[derive(Debug, Copy, Clone)]
pub struct S2CMatchStartBody {
    pub clock: OptionalClock,
    pub variant: i64,
    pub match_id: u64,
    pub color: Color,
    pub message_id: u64,
}
#[derive(Debug, Copy, Clone)]
pub struct C2SOrS2CActionBody {
    pub action_type: ActionType,
    pub color: Color,
    pub message_id: u64,
    pub src_l: i64,
    pub src_t: i64,
    pub src_board_color: Color,
    pub src_y: i64,
    pub src_x: i64,
    pub dst_l: i64,
    pub dst_t: i64,
    pub dst_board_color: Color,
    pub dst_y: i64,
    pub dst_x: i64,
}
#[derive(Debug, Copy, Clone)]
pub struct S2CMatchListBody {
    pub color: OptionalClock,
    pub clock: OptionalClock,
    pub variant: i64,
    pub passcode: i64,
    pub is_host: bool,
    pub public_matches: [S2CMatchListPublicMatch; 13],
    pub public_matches_count: u64,
    pub server_history_matches: [S2CMatchListServerHistoryMatch; 13],
    pub server_history_matches_count: u64,
}
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct S2CMatchListPublicMatch {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: i64,
    pub passcode: i64,
}
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct S2CMatchListServerHistoryMatch {
    pub status: HistoryMatchStatus,
    pub clock: OptionalClock,
    pub variant: i64,
    pub visibility: Visibility,
    pub seconds_passed: i64,
}

impl Message {
    pub fn message_type(&self) -> MessageType {
        match self {
            Message::C2SGreet(_) => MessageType::C2SGreet,
            Message::S2CGreet => MessageType::S2CGreet,
            Message::C2SMatchCreateOrJoin(_) => MessageType::C2SMatchCreateOrJoin,
            Message::S2CMatchCreateOrJoinResult(_) => MessageType::S2CMatchCreateOrJoinResult,
            Message::C2SMatchCancel => MessageType::C2SMatchCancel,
            Message::S2CMatchCancelResult(_) => MessageType::S2CMatchCancelResult,
            Message::S2CMatchStart(_) => MessageType::S2CMatchStart,
            Message::S2COpponentLeft => MessageType::S2COpponentLeft,
            Message::C2SForfeit => MessageType::C2SForfeit,
            Message::C2SOrS2CAction(_) => MessageType::C2SOrS2CAction,
            Message::C2SMatchListRequest => MessageType::C2SMatchListRequest,
            Message::S2CMatchList(_) => MessageType::S2CMatchList,
        }
    }

    pub fn legal_length(&self) -> usize {
        self.message_type().legal_length()
    }

    pub fn pack(&self) -> Result<Bytes> {
        let mut bytes = BytesMut::new();
        write_i64_le(&mut bytes, self.message_type() as i64);
        match self {
            Message::S2CGreet => {
                write_i64_le(&mut bytes, 1); // version, unconfirmed
                for _ in 0..5 {
                    write_i64_le(&mut bytes, 0); // unknown
                }
            }
            Message::S2CMatchCreateOrJoinResult(body) => {
                match body {
                    S2CMatchCreateOrJoinResultBody::Success(body) => {
                        write_i64_le(&mut bytes, 1); // success
                        write_i64_le(&mut bytes, 0); // success
                        write_i64_le(&mut bytes, body.color as i64);
                        write_i64_le(&mut bytes, body.clock as i64);
                        write_i64_le(&mut bytes, body.variant);
                        write_i64_le(&mut bytes, body.visibility as i64);
                        write_i64_le(&mut bytes, body.passcode);
                    }
                    S2CMatchCreateOrJoinResultBody::Failed => {
                        write_i64_le(&mut bytes, 0); // failed
                        write_i64_le(&mut bytes, 1); // failed
                        for _ in 0..4 {
                            write_i64_le(&mut bytes, 0);
                        }
                        write_i64_le(&mut bytes, -1);
                    }
                };
            }
            Message::S2CMatchCancelResult(body) => {
                write_i64_le(
                    &mut bytes,
                    match body {
                        S2CMatchCancelResultBody::Success => 1,
                        S2CMatchCancelResultBody::Failed => 0,
                    },
                );
            }
            Message::S2CMatchStart(body) => {
                write_i64_le(&mut bytes, body.clock as i64);
                write_i64_le(&mut bytes, body.variant);
                write_u64_le(&mut bytes, body.match_id);
                write_i64_le(&mut bytes, body.color as i64);
                write_u64_le(&mut bytes, body.message_id);
            }
            Message::S2COpponentLeft => {
                bytes.extend_from_slice(&[0]); // unknown
            }
            Message::C2SOrS2CAction(body) => {
                write_i64_le(&mut bytes, body.action_type as i64);
                write_i64_le(&mut bytes, body.color as i64);
                write_u64_le(&mut bytes, body.message_id);
                write_i64_le(&mut bytes, body.src_l);
                write_i64_le(&mut bytes, body.src_t);
                write_i64_le(&mut bytes, body.src_board_color as i64);
                write_i64_le(&mut bytes, body.src_y);
                write_i64_le(&mut bytes, body.src_x);
                write_i64_le(&mut bytes, body.dst_l);
                write_i64_le(&mut bytes, body.dst_t);
                write_i64_le(&mut bytes, body.dst_board_color as i64);
                write_i64_le(&mut bytes, body.dst_y);
                write_i64_le(&mut bytes, body.dst_x);
            }
            Message::S2CMatchList(body) => {
                write_i64_le(&mut bytes, 1); // unknown
                write_i64_le(&mut bytes, body.color as i64);
                write_i64_le(&mut bytes, body.clock as i64);
                write_i64_le(&mut bytes, body.variant as i64);
                write_i64_le(&mut bytes, body.passcode);
                write_i64_le(&mut bytes, if body.is_host { 1 } else { 0 });
                for i in 0..13 {
                    write_i64_le(&mut bytes, body.public_matches[i].color as i64);
                    write_i64_le(&mut bytes, body.public_matches[i].clock as i64);
                    write_i64_le(&mut bytes, body.public_matches[i].variant);
                    write_i64_le(&mut bytes, body.public_matches[i].passcode);
                }
                write_u64_le(&mut bytes, body.public_matches_count);
                for i in 0..13 {
                    write_i64_le(&mut bytes, body.server_history_matches[i].status as i64);
                    write_i64_le(&mut bytes, body.server_history_matches[i].clock as i64);
                    write_i64_le(&mut bytes, body.server_history_matches[i].variant);
                    write_i64_le(&mut bytes, body.server_history_matches[i].visibility as i64);
                    write_i64_le(&mut bytes, body.server_history_matches[i].seconds_passed);
                }
                write_u64_le(&mut bytes, body.server_history_matches_count);
            }
            _ => {
                return err_invalid_data!(
                    "Message type {:?} shouldn't be packed.",
                    self.message_type()
                );
            }
        };

        // check message length
        if bytes.len() != self.legal_length() {
            return err_invalid_data!(
                "Message of type {:?} should be of length {}, not {}.",
                self.message_type(),
                self.legal_length(),
                bytes.len()
            );
        }
        Ok(bytes.into())
    }

    pub fn unpack(mut bytes: BytesMut) -> Result<Message> {
        let length = bytes.len();
        let message_type: MessageType = try_i64_to_enum(read_i64_le(&mut bytes))?;

        // check message length
        if length != message_type.legal_length() {
            return err_invalid_data!(
                "Message of type {:?} should be of length {}, not {}.",
                message_type,
                message_type.legal_length(),
                length
            );
        }

        match message_type {
            MessageType::C2SGreet => {
                let version1 = read_i64_le(&mut bytes);
                let version2 = read_i64_le(&mut bytes);
                Ok(Message::C2SGreet(C2SGreetBody { version1, version2 }))
            }
            MessageType::C2SMatchCreateOrJoin => {
                let color = try_i64_to_enum(read_i64_le(&mut bytes))?;
                let clock = try_i64_to_enum(read_i64_le(&mut bytes))?;
                let variant = read_i64_le(&mut bytes);
                let visibility = try_i64_to_enum(read_i64_le(&mut bytes))?;
                let passcode = read_i64_le(&mut bytes);
                Ok(Message::C2SMatchCreateOrJoin(C2SMatchCreateOrJoinBody {
                    color,
                    clock,
                    variant,
                    visibility,
                    passcode,
                }))
            }
            MessageType::C2SMatchCancel => Ok(Message::C2SMatchCancel),
            MessageType::C2SForfeit => Ok(Message::C2SForfeit),
            MessageType::C2SOrS2CAction => {
                let action_type = try_i64_to_enum(read_i64_le(&mut bytes))?;
                let color = try_i64_to_enum(read_i64_le(&mut bytes))?;
                let message_id = read_u64_le(&mut bytes);
                let src_l = read_i64_le(&mut bytes);
                let src_t = read_i64_le(&mut bytes);
                let src_board_color = try_i64_to_enum(read_i64_le(&mut bytes))?;
                let src_x = read_i64_le(&mut bytes);
                let src_y = read_i64_le(&mut bytes);
                let dst_l = read_i64_le(&mut bytes);
                let dst_t = read_i64_le(&mut bytes);
                let dst_board_color = try_i64_to_enum(read_i64_le(&mut bytes))?;
                let dst_x = read_i64_le(&mut bytes);
                let dst_y = read_i64_le(&mut bytes);
                Ok(Message::C2SOrS2CAction(C2SOrS2CActionBody {
                    action_type,
                    color,
                    message_id,
                    src_l,
                    src_t,
                    src_board_color,
                    src_y,
                    src_x,
                    dst_l,
                    dst_t,
                    dst_board_color,
                    dst_y,
                    dst_x,
                }))
            }
            MessageType::C2SMatchListRequest => Ok(Message::C2SMatchListRequest),
            _ => err_invalid_data!("Message type {:?} shouldn't be unpacked.", message_type),
        }
    }
}

pub struct MessageIO {
    framed: Framed<TcpStream, LengthDelimitedCodec>,
}

impl MessageIO {
    pub fn new(stream: TcpStream) -> Self {
        MessageIO {
            framed: LengthDelimitedCodec::builder()
                .little_endian()
                .length_field_type::<u64>()
                .max_frame_length(MESSAGE_LENGTH_MAX)
                .new_framed(stream),
        }
    }

    pub async fn get(&mut self) -> Result<Message> {
        match self.framed.next().await {
            Some(Ok(message)) => Message::unpack(message),
            Some(Err(e)) => Err(e),
            None => err_disconnected!(),
        }
    }

    pub async fn put(&mut self, message: Message) -> Result<()> {
        match message.pack() {
            Ok(message) => self.framed.feed(message).await,
            Err(e) => Err(e),
        }
    }

    pub async fn flush(&mut self) -> Result<()> {
        self.framed.flush().await
    }
}

pub fn read_i64_le(bytes: &mut BytesMut) -> i64 {
    LittleEndian::read_i64(&bytes.split_to(8)[..])
}

pub fn read_u64_le(bytes: &mut BytesMut) -> u64 {
    LittleEndian::read_u64(&bytes.split_to(8)[..])
}

pub fn write_i64_le(bytes: &mut BytesMut, n: i64) {
    let mut buffer = [0; 8];
    LittleEndian::write_i64(&mut buffer[..], n);
    bytes.extend_from_slice(&buffer[..]);
}

pub fn write_u64_le(bytes: &mut BytesMut, n: u64) {
    let mut buffer = [0; 8];
    LittleEndian::write_u64(&mut buffer[..], n);
    bytes.extend_from_slice(&buffer[..]);
}

pub fn try_i64_to_enum<T: num::FromPrimitive>(v: i64) -> Result<T> {
    match T::from_i64(v) {
        Some(v) => Ok(v),
        None => err_invalid_data!(
            "Unknown value {} for enum type {}.",
            v,
            std::any::type_name::<T>()
        ),
    }
}

pub fn generate_random_passcode_internal() -> i64 {
    rand::thread_rng().gen_range(0..=2985983) // kkkkkk = 2985983
}

pub async fn generate_random_passcode_internal_with_exceptions(
    matches: &Mutex<HashMap<i64, InternalMatch>>,
) -> i64 {
    loop {
        let v = generate_random_passcode_internal();
        if !matches.lock().await.contains_key(&v) {
            return v;
        }
    }
}
