use super::{MeshAddr, ModelIdentifier, Opcode, Status};
use nom::{
    combinator::peek,
    number::complete::{be_u16, le_u16, u8},
};
use std::convert::TryFrom;

pub(super) fn parse<T, F>(buf: &[u8], parser: F) -> Option<T>
where
    T: super::messages::ParsePacket,
    F: Fn(&[u8]) -> nom::IResult<&[u8], T>,
{
    match parser(buf) {
        Ok((b"", ret)) => Some(ret),
        _ => None,
    }
}

pub(super) fn status(buf: &[u8]) -> nom::IResult<&[u8], Status> {
    let (buf, n) = u8(buf)?;
    Ok((buf, Status::from(n)))
}

pub(super) fn model_identifier(buf: &[u8]) -> nom::IResult<&[u8], ModelIdentifier> {
    if buf.len() == 4 {
        let (buf, company_id) = le_u16(buf)?;
        let (buf, model_id) = le_u16(buf)?;
        Ok((
            buf,
            ModelIdentifier::Vendor {
                company_id,
                model_id,
            },
        ))
    } else if buf.len() == 2 {
        let (buf, sig_id) = le_u16(buf)?;
        Ok((buf, ModelIdentifier::Sig(sig_id)))
    } else {
        // FIXME: make a proper error
        let a = nom::error::Error::new(buf, nom::error::ErrorKind::Count);
        Err(nom::Err::Failure(a))
    }
}

pub(super) fn mesh_addr(buf: &[u8]) -> nom::IResult<&[u8], MeshAddr> {
    let (buf, addr) = le_u16(buf)?;
    Ok((buf, MeshAddr::from(addr)))
}

fn take_buf<const N: usize>(buf: &[u8]) -> nom::IResult<&[u8], [u8; N]> {
    let (buf, ret) = nom::bytes::complete::take(N)(buf)?;
    Ok((buf, <[u8; N]>::try_from(ret).unwrap()))
}

pub(super) fn unpack_key_index(buf: &[u8]) -> nom::IResult<&[u8], (u16, u16)> {
    let (buf, keys) = take_buf::<3>(buf)?;

    let a = (((keys[1] & 0xf0) as u16) << 4) | keys[0] as u16;
    let b = ((keys[2] as u16) << 4) | ((keys[1] & 0x0f) as u16);

    Ok((buf, (a, b)))
}

pub(super) fn opcode(buf: &[u8]) -> nom::IResult<&[u8], super::Opcode> {
    let (buf, first) = peek(u8)(buf)?;

    if first & 0xc0 == 0xc0 {
        let (buf, code) = u8(buf)?;
        let (buf, vendor) = le_u16(buf)?;
        Ok((buf, Opcode::Vendor { code, vendor }))
    } else if first & 0x80 == 0x80 {
        let (buf, opcode) = be_u16(buf)?;
        Ok((buf, Opcode::Sig2(opcode)))
    } else {
        let (buf, opcode) = u8(buf)?;
        Ok((buf, Opcode::Sig1(opcode)))
    }
}
