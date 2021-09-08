mod messages;
mod parse;
mod put;
pub mod sig;
pub mod vendor;
pub use messages::{Packet, ParsePacket, ToPacket};

use crate::util::Hex;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Sig1(u8),
    Sig2(u16),
    Vendor { code: u8, vendor: u16 },
}

impl Opcode {
    pub fn new_vendor(op: u8, vendor: u16) -> Self {
        Opcode::Vendor {
            code: 0xc0 | op,
            vendor,
        }
    }
}

impl std::fmt::Debug for Opcode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Opcode::Sig1(n) => {
                write!(f, "Sig1(0x{:02x})", n)
            }
            Opcode::Sig2(n) => {
                write!(f, "Sig2(0x{:04x})", n)
            }
            Opcode::Vendor { code, vendor } => {
                write!(
                    f,
                    "Vendor{{ code: 0x{:02x} vendor: 0x{:04x} }}",
                    code, vendor
                )
            }
        }
    }
}

#[derive(Debug)]
pub struct Message {
    pub opcode: Opcode,
    pub payload: Vec<u8>,
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Packet is empty")]
    Empty,
    #[error("Packet opcode is malformed")]
    Malformed,
}

impl Message {
    pub fn parse(buf: &[u8]) -> Result<Self, Error> {
        let (payload, opcode) = parse::opcode(buf).map_err(|_| Error::Malformed)?;
        Ok(Self {
            opcode,
            // NOTE: would allocate even when taken by value because the opcode
            // is in the first few bytes
            payload: payload.to_vec(),
        })
    }
}

#[derive(
    serde::Serialize,
    serde::Deserialize,
    zvariant::derive::Value,
    zvariant::derive::Type,
    derive_more::From,
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
)]
pub struct MeshAddr(u16);

impl MeshAddr {
    pub fn as_u16(self) -> u16 {
        self.0
    }
}

impl std::fmt::Display for MeshAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // NOTE: 0x prefix counts as string to zero pad so 4 for hex number and 2 for 0x prefix
        write!(f, "{:#06x}", self.0)
    }
}

#[derive(num_enum::FromPrimitive, Clone, Copy, Debug)]
#[repr(u8)]
pub enum Status {
    Success = 0x00,
    InvalidAddress = 0x01,
    InvalidModel = 0x02,
    InvalidAppKeyIndex = 0x03,
    InvalidNetKeyIndex = 0x04,
    InsufficientResources = 0x05,
    KeyIndexAlreadyStored = 0x06,
    InvalidPublishParameters = 0x07,
    NotASubscribeModel = 0x08,
    StorageFailure = 0x09,
    FeatureNotSupported = 0x0A,
    CannotUpdate = 0x0B,
    CannotRemove = 0x0C,
    CannotBind = 0x0D,
    TemporarilyUnableToChangeState = 0x0E,
    CannotSet = 0x0F,
    UnspecifiedError = 0x10,
    InvalidBinding = 0x11,
    #[num_enum(default)]
    RFU,
}

impl Status {
    pub fn is_ok(self) -> bool {
        match self {
            Status::Success => true,
            _ => false,
        }
    }
}

#[derive(Copy, Clone)]
pub enum ModelIdentifier {
    Sig(u16),
    Vendor { company_id: u16, model_id: u16 },
}

impl std::fmt::Debug for ModelIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sig(arg0) => f
                .debug_tuple("Sig")
                .field(&Hex(&arg0.to_ne_bytes()))
                .finish(),
            Self::Vendor {
                company_id,
                model_id,
            } => f
                .debug_struct("Vendor")
                .field("company_id", &Hex(&company_id.to_ne_bytes()))
                .field("model_id", &Hex(&model_id.to_ne_bytes()))
                .finish(),
        }
    }
}
