use crate::{MsgpackError, Result};

pub fn write_bool(out: &mut Vec<u8>, value: bool) {
    out.push(if value { 0xc3 } else { 0xc2 });
}

pub fn write_i32(out: &mut Vec<u8>, value: i32) {
    write_i64(out, value as i64);
}

pub fn write_i8(out: &mut Vec<u8>, value: i8) {
    out.push(0xd0);
    out.push(value as u8);
}

pub fn write_i16(out: &mut Vec<u8>, value: i16) {
    out.push(0xd1);
    out.extend_from_slice(&value.to_be_bytes());
}

pub fn write_i64(out: &mut Vec<u8>, value: i64) {
    // 与 msgpack-java `MessagePacker.packLong` 的行为对齐：非负数走 uint 系列
    // (0xcc/0xcd/0xce/0xcf)，负数走 int 系列 (0xd0/0xd1/0xd2/0xd3)。
    // Vivo 手表端的 msgpack 解码器对 marker 敏感，正数若用 0xd2 (int32)
    // 会被部分命令静默丢弃（如 BID 1 / CID 1 DialInstallBleRequest）。
    if value >= 0 {
        let v = value as u64;
        if v <= 0x7f {
            out.push(v as u8);
        } else if v <= u8::MAX as u64 {
            out.push(0xcc);
            out.push(v as u8);
        } else if v <= u16::MAX as u64 {
            out.push(0xcd);
            out.extend_from_slice(&(v as u16).to_be_bytes());
        } else if v <= u32::MAX as u64 {
            out.push(0xce);
            out.extend_from_slice(&(v as u32).to_be_bytes());
        } else {
            out.push(0xcf);
            out.extend_from_slice(&v.to_be_bytes());
        }
    } else if value >= -32 {
        out.push(value as i8 as u8);
    } else if value >= i8::MIN as i64 {
        out.push(0xd0);
        out.push(value as i8 as u8);
    } else if value >= i16::MIN as i64 {
        out.push(0xd1);
        out.extend_from_slice(&(value as i16).to_be_bytes());
    } else if value >= i32::MIN as i64 {
        out.push(0xd2);
        out.extend_from_slice(&(value as i32).to_be_bytes());
    } else {
        out.push(0xd3);
        out.extend_from_slice(&value.to_be_bytes());
    }
}

pub fn write_f32(out: &mut Vec<u8>, value: f32) {
    out.push(0xca);
    out.extend_from_slice(&value.to_be_bytes());
}

pub fn write_f64(out: &mut Vec<u8>, value: f64) {
    out.push(0xcb);
    out.extend_from_slice(&value.to_be_bytes());
}

pub fn write_str(out: &mut Vec<u8>, value: &str) -> Result<()> {
    let len = value.len();
    if len <= 31 {
        out.push(0xa0 | len as u8);
    } else if len <= u8::MAX as usize {
        out.push(0xd9);
        out.push(len as u8);
    } else if len <= u16::MAX as usize {
        out.push(0xda);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        let len = u32::try_from(len)?;
        out.push(0xdb);
        out.extend_from_slice(&len.to_be_bytes());
    }
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

pub fn write_bin(out: &mut Vec<u8>, value: &[u8]) -> Result<()> {
    let len = value.len();
    if len <= u8::MAX as usize {
        out.push(0xc4);
        out.push(len as u8);
    } else if len <= u16::MAX as usize {
        out.push(0xc5);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        let len = u32::try_from(len)?;
        out.push(0xc6);
        out.extend_from_slice(&len.to_be_bytes());
    }
    out.extend_from_slice(value);
    Ok(())
}

pub fn write_array_len(out: &mut Vec<u8>, len: usize) -> Result<()> {
    if len <= 15 {
        out.push(0x90 | len as u8);
    } else if len <= u16::MAX as usize {
        out.push(0xdc);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        let len = u32::try_from(len)?;
        out.push(0xdd);
        out.extend_from_slice(&len.to_be_bytes());
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct MsgpackReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> MsgpackReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn has_next(&self) -> bool {
        self.pos < self.buf.len()
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn peek_marker(&self) -> Option<u8> {
        self.buf.get(self.pos).copied()
    }

    fn read_u8(&mut self) -> Result<u8> {
        let byte = *self
            .buf
            .get(self.pos)
            .ok_or(MsgpackError::Msgpack("unexpected end of buffer"))?;
        self.pos += 1;
        Ok(byte)
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(MsgpackError::Msgpack("length overflow"))?;
        if end > self.buf.len() {
            return Err(MsgpackError::Msgpack("unexpected end of buffer"));
        }
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub fn read_bool(&mut self) -> Result<bool> {
        match self.read_u8()? {
            0xc2 => Ok(false),
            0xc3 => Ok(true),
            _ => Err(MsgpackError::Msgpack("expected bool")),
        }
    }

    pub fn read_i32(&mut self) -> Result<i32> {
        Ok(i32::try_from(self.read_i64()?)?)
    }

    pub fn read_i8(&mut self) -> Result<i8> {
        Ok(i8::try_from(self.read_i64()?)?)
    }

    pub fn read_i16(&mut self) -> Result<i16> {
        Ok(i16::try_from(self.read_i64()?)?)
    }

    pub fn read_i64(&mut self) -> Result<i64> {
        let marker = self.read_u8()?;
        match marker {
            0x00..=0x7f => Ok(marker as i64),
            0xcc => Ok(self.read_u8()? as i64),
            0xcd => Ok(u16::from_be_bytes(self.read_array()?) as i64),
            0xce => Ok(u32::from_be_bytes(self.read_array()?) as i64),
            0xcf => i64::try_from(u64::from_be_bytes(self.read_array()?)).map_err(Into::into),
            0xd0 => Ok(i8::from_be_bytes([self.read_u8()?]) as i64),
            0xd1 => Ok(i16::from_be_bytes(self.read_array()?) as i64),
            0xd2 => Ok(i32::from_be_bytes(self.read_array()?) as i64),
            0xd3 => Ok(i64::from_be_bytes(self.read_array()?)),
            0xe0..=0xff => Ok((marker as i8) as i64),
            _ => Err(MsgpackError::Msgpack("expected integer")),
        }
    }

    pub fn read_f32(&mut self) -> Result<f32> {
        let marker = self.read_u8()?;
        match marker {
            0xca => Ok(f32::from_be_bytes(self.read_array()?)),
            0xcb => Ok(f64::from_be_bytes(self.read_array()?) as f32),
            _ => Err(MsgpackError::Msgpack("expected float")),
        }
    }

    pub fn read_f64(&mut self) -> Result<f64> {
        let marker = self.read_u8()?;
        match marker {
            0xca => Ok(f32::from_be_bytes(self.read_array()?) as f64),
            0xcb => Ok(f64::from_be_bytes(self.read_array()?)),
            _ => Err(MsgpackError::Msgpack("expected float")),
        }
    }

    pub fn read_str(&mut self) -> Result<String> {
        let marker = self.read_u8()?;
        let len = match marker {
            0xa0..=0xbf => (marker & 0x1f) as usize,
            0xd9 => self.read_u8()? as usize,
            0xda => u16::from_be_bytes(self.read_array()?) as usize,
            0xdb => usize::try_from(u32::from_be_bytes(self.read_array()?))?,
            _ => return Err(MsgpackError::Msgpack("expected string")),
        };
        let bytes = self.read_exact(len)?;
        Ok(std::str::from_utf8(bytes)?.to_string())
    }

    pub fn read_bin(&mut self) -> Result<Vec<u8>> {
        let marker = self.read_u8()?;
        let len = match marker {
            0xc4 => self.read_u8()? as usize,
            0xc5 => u16::from_be_bytes(self.read_array()?) as usize,
            0xc6 => usize::try_from(u32::from_be_bytes(self.read_array()?))?,
            _ => return Err(MsgpackError::Msgpack("expected binary")),
        };
        Ok(self.read_exact(len)?.to_vec())
    }

    pub fn read_array_len(&mut self) -> Result<usize> {
        let marker = self.read_u8()?;
        match marker {
            0x90..=0x9f => Ok((marker & 0x0f) as usize),
            0xdc => Ok(u16::from_be_bytes(self.read_array()?) as usize),
            0xdd => Ok(usize::try_from(u32::from_be_bytes(self.read_array()?))?),
            _ => Err(MsgpackError::Msgpack("expected array")),
        }
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let bytes = self.read_exact(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }
}
