use super::Opcode;
use nom::number::complete::{le_i16, le_u16, le_u32};

pub struct BleWsReadSensor;

impl super::Packet for BleWsReadSensor {
    fn opcode() -> super::Opcode {
        super::Opcode::new_vendor(0x00, crate::mesh::COMPANY_ID)
    }
}

impl super::ToPacket for BleWsReadSensor {
    fn put_payload(&self, _buf: &mut Vec<u8>) {}
}

#[derive(Debug)]
pub struct BleWsSensorValues {
    pub temperature: i16,
    pub humidity: u16,
    pub pressure: u32,
}

impl super::Packet for BleWsSensorValues {
    fn opcode() -> super::Opcode {
        Opcode::new_vendor(0x01, crate::mesh::COMPANY_ID)
    }
}

impl super::ParsePacket for BleWsSensorValues {
    fn parse_payload(buf: &[u8]) -> Option<Self> {
        super::parse::parse(buf, |buf| {
            let (buf, temperature) = le_i16(buf)?;
            let (buf, humidity) = le_u16(buf)?;
            let (buf, pressure) = le_u32(buf)?;
            Ok((
                buf,
                Self {
                    temperature,
                    humidity,
                    pressure,
                },
            ))
        })
    }
}
