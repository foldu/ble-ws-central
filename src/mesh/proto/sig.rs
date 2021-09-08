use super::{
    messages::{Packet, ParsePacket, ToPacket},
    parse::{mesh_addr, model_identifier, parse, status, unpack_key_index},
    put::put_model_id,
    MeshAddr, ModelIdentifier, Opcode, Status,
};
use bytes::BufMut;
use nom::number::complete::le_u16;

pub struct ConfigModelAppBind {
    pub element_address: MeshAddr,
    pub app_key_index: u16,
    pub model_identifier: ModelIdentifier,
}

impl Packet for ConfigModelAppBind {
    fn opcode() -> Opcode {
        Opcode::Sig2(0x803d)
    }
}

impl ToPacket for ConfigModelAppBind {
    fn put_payload(&self, buf: &mut Vec<u8>) {
        buf.put_u16_le(self.element_address.as_u16());
        buf.put_u16_le(self.app_key_index);
        put_model_id(buf, self.model_identifier);
    }
}

pub struct ConfigModelAppStatus {
    pub status: super::Status,
    pub element_address: super::MeshAddr,
    pub app_key_index: u16,
    pub model_identifier: ModelIdentifier,
}

impl Packet for ConfigModelAppStatus {
    fn opcode() -> Opcode {
        Opcode::Sig2(0x803e)
    }
}

impl ParsePacket for ConfigModelAppStatus {
    fn parse_payload(buf: &[u8]) -> Option<Self> {
        parse(buf, |buf| {
            let (buf, status) = status(buf)?;
            let (buf, element_address) = mesh_addr(buf)?;
            let (buf, app_key_index) = le_u16(buf)?;
            let (buf, model_identifier) = model_identifier(buf)?;
            Ok((
                buf,
                Self {
                    status,
                    element_address,
                    app_key_index,
                    model_identifier,
                },
            ))
        })
    }
}

pub struct ConfigAppKeyStatus {
    pub status: Status,
    pub net_key_index: u16,
    pub app_key_index: u16,
}

impl Packet for ConfigAppKeyStatus {
    fn opcode() -> Opcode {
        Opcode::Sig2(0x8003)
    }
}

impl ParsePacket for ConfigAppKeyStatus {
    fn parse_payload(buf: &[u8]) -> Option<Self> {
        parse(buf, |buf| {
            let (buf, status) = status(buf)?;
            let (buf, (net_key_index, app_key_index)) = unpack_key_index(buf)?;
            Ok((
                buf,
                Self {
                    status,
                    net_key_index,
                    app_key_index,
                },
            ))
        })
    }
}
