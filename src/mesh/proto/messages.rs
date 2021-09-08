use super::Opcode;
use bytes::BufMut;

const MESH_PACKET_MAX_SIZE: usize = 380;

pub trait Packet {
    fn opcode() -> Opcode;
}

pub trait ParsePacket: Sized + Packet {
    fn parse_payload(buf: &[u8]) -> Option<Self>;
}

pub trait ToPacket: Packet {
    fn put_payload(&self, buf: &mut Vec<u8>);

    fn encode(&self) -> Vec<u8> {
        let mut ret = Vec::new();
        match Self::opcode() {
            Opcode::Sig1(n) => ret.push(n),
            Opcode::Sig2(n) => ret.put_u16(n),
            Opcode::Vendor { code, vendor } => {
                ret.put_u8(code);
                ret.put_u16_le(vendor);
            }
        }
        self.put_payload(&mut ret);
        assert!(ret.len() < MESH_PACKET_MAX_SIZE);
        ret
    }
}
