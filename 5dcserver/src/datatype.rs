use byteorder::{ByteOrder, LittleEndian};
use bytes::{Bytes, BytesMut};
use enum_primitive::{enum_from_primitive, enum_from_primitive_impl, enum_from_primitive_impl_ty};
use std::io::Result;
use tokio_util::codec::{Decoder, Encoder};

pub const MESSAGE_LENGTH_MAX: usize = 4096; // >= 1008, prevent attacks

macro_rules! error_invalid_data {
    ( $reason:expr ) => {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            $reason,
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

enum_from_primitive! {
    #[repr(i64)]
    #[derive(Debug, Copy, Clone, PartialEq)]
    pub enum Color {
        White = 0,
        Black = 1
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
pub struct Match {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: u64,
    pub visibility: Visibility,
    pub passcode: u64,
}

#[derive(Debug, Copy, Clone)]
pub struct PublicMatch {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: u64,
    pub passcode: u64,
}

#[derive(Debug, Copy, Clone)]
pub struct PrivateMatch {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: u64,
    pub passcode: u64,
}

#[derive(Debug, Copy, Clone)]
pub struct ServerHistoryMatch {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: u64,
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
        S2CMatchCreateOrJoinSuccess = 4,
        C2SMatchCancel = 5,
        S2CMatchCancelSuccess = 6,
        S2CMatchStart = 7,

        S2COpponentLeft = 9,
        C2SForfeit = 10,
        C2SOrS2CAction = 11,
        C2SMatchListRequest = 12,
        S2CMatchList = 13
    }
}

pub struct MessageCodec {}
impl Decoder for MessageCodec {
    type Item = (MessageType, BytesMut);
    type Error = std::io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>> {
        if src.len() < 8 {
            return Ok(None);
        }
        let length = LittleEndian::read_u64(&src[0..7]) as usize;
        if length > MESSAGE_LENGTH_MAX {
            return error_invalid_data!(format!("Message of length {} is too large.", length));
        }
        if src.len() < 8 + length {
            src.reserve(8 + length - src.len());
            return Ok(None);
        }

        let length = read_u64_le(src);
        let mut message_bytes = src.split_to(length as usize);
        let message_type = try_i64_to_enum(read_i64_le(&mut message_bytes))?;

        // check message length
        let legal_length = match message_type {
            MessageType::C2SGreet => 56,
            MessageType::S2CGreet => 56,
            MessageType::C2SMatchCreateOrJoin => 48,
            MessageType::S2CMatchCreateOrJoinSuccess => 64,
            MessageType::C2SMatchCancel => 9,
            MessageType::S2CMatchCancelSuccess => 16,
            MessageType::S2CMatchStart => 48,
            MessageType::S2COpponentLeft => 9,
            MessageType::C2SForfeit => 9,
            MessageType::C2SOrS2CAction => 112,
            MessageType::C2SMatchListRequest => 9,
            MessageType::S2CMatchList => 1008,
        };
        if length != legal_length {
            error_invalid_data!(format!(
                "Message of type {:?} should be of length {}, not {}.",
                message_type, legal_length, length
            ))
        } else {
            Ok(Some((message_type, message_bytes)))
        }
    }
}

impl Encoder<(MessageType, Bytes)> for MessageCodec {
    type Error = std::io::Error;

    fn encode(&mut self, item: (MessageType, Bytes), dst: &mut BytesMut) -> Result<()> {
        let (message_type, message_bytes) = item;
        let length = message_bytes.len();
        if length > MESSAGE_LENGTH_MAX {
            return error_invalid_data!(format!("Message of length {} is too large.", length));
        }
        dst.reserve(16 + message_bytes.len());
        let mut buffer = [0; 8];
        LittleEndian::write_u64(&mut buffer, 8 + message_bytes.len() as u64);
        dst.extend(buffer);
        LittleEndian::write_u64(&mut buffer, message_type as u64);
        dst.extend(buffer);
        dst.extend(message_bytes);
        Ok(())
    }
}

// unknown fields omitted
pub enum Message {
    C2SGreet(C2SGreetBody),
    S2CGreet,
    C2SMatchCreateOrJoin(C2SMatchCreateOrJoinBody),
    S2CMatchCreateOrJoinSuccess(S2CMatchCreateOrJoinSuccessBody),
    C2SMatchCancel,
    S2CMatchCancelSuccess(S2CMatchCancelSuccessBody),
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
pub struct S2CMatchCreateOrJoinSuccessBody {
    pub color: OptionalColorWithRandom,
    pub clock: OptionalClock,
    pub variant: i64,
    pub visibility: Visibility,
    pub passcode: i64,
}
#[derive(Debug, Copy, Clone)]
pub struct S2CMatchCancelSuccessBody {
    pub cancel_count: i64,
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
    pub fn pack(&self) -> (MessageType, BytesMut) {
        let mut bytes = BytesMut::new();
        match self {
            Message::S2CGreet => {
                write_i64_le(&mut bytes, 1); // version, unconfirmed
                for _ in 0..5 {
                    write_i64_le(&mut bytes, 0); // unknown
                }
                (MessageType::S2CGreet, bytes)
            }
            Message::S2CMatchCreateOrJoinSuccess(body) => {
                write_i64_le(&mut bytes, 1); // unknown
                write_i64_le(&mut bytes, 0); // unknown
                write_i64_le(&mut bytes, body.color as i64);
                write_i64_le(&mut bytes, body.clock as i64);
                write_i64_le(&mut bytes, body.variant);
                write_i64_le(&mut bytes, body.visibility as i64);
                write_i64_le(&mut bytes, body.passcode);
                (MessageType::S2CGreet, bytes)
            }
            Message::S2CMatchCancelSuccess(body) => {
                write_i64_le(&mut bytes, body.cancel_count);
                (MessageType::S2CGreet, bytes)
            }
            Message::S2CMatchStart(body) => {
                write_i64_le(&mut bytes, body.clock as i64);
                write_i64_le(&mut bytes, body.variant);
                write_u64_le(&mut bytes, body.match_id);
                write_i64_le(&mut bytes, body.color as i64);
                write_u64_le(&mut bytes, body.message_id);
                (MessageType::S2CGreet, bytes)
            }
            Message::S2COpponentLeft => {
                bytes.extend_from_slice(&[0]); // unknown
                (MessageType::S2CGreet, bytes)
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
                (MessageType::S2CGreet, bytes)
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
                (MessageType::S2CGreet, bytes)
            }
            _ => unimplemented!(),
        }
    }

    pub fn unpack(message_type: &MessageType, mut message_bytes: BytesMut) -> Result<Message> {
        match message_type {
            MessageType::C2SGreet => {
                let version1 = read_i64_le(&mut message_bytes);
                let version2 = read_i64_le(&mut message_bytes);
                Ok(Message::C2SGreet(C2SGreetBody { version1, version2 }))
            }
            MessageType::C2SMatchCreateOrJoin => {
                let color = try_i64_to_enum(read_i64_le(&mut message_bytes))?;
                let clock = try_i64_to_enum(read_i64_le(&mut message_bytes))?;
                let variant = read_i64_le(&mut message_bytes);
                let visibility = try_i64_to_enum(read_i64_le(&mut message_bytes))?;
                let passcode = read_i64_le(&mut message_bytes);
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
                let action_type = try_i64_to_enum(read_i64_le(&mut message_bytes))?;
                let color = try_i64_to_enum(read_i64_le(&mut message_bytes))?;
                let message_id = read_u64_le(&mut message_bytes);
                let src_l = read_i64_le(&mut message_bytes);
                let src_t = read_i64_le(&mut message_bytes);
                let src_board_color = try_i64_to_enum(read_i64_le(&mut message_bytes))?;
                let src_x = read_i64_le(&mut message_bytes);
                let src_y = read_i64_le(&mut message_bytes);
                let dst_l = read_i64_le(&mut message_bytes);
                let dst_t = read_i64_le(&mut message_bytes);
                let dst_board_color = try_i64_to_enum(read_i64_le(&mut message_bytes))?;
                let dst_x = read_i64_le(&mut message_bytes);
                let dst_y = read_i64_le(&mut message_bytes);
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
            _ => unimplemented!(),
        }
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
        None => error_invalid_data!(format!(
            "Unknown value {} for enum type {}.",
            v,
            std::any::type_name::<T>()
        )),
    }
}
