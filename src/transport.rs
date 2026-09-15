//! Versioned binary framing shared by the future Linux server and Windows client.
//!
//! The control-plane daemon still exposes a deliberately small text bootstrap
//! protocol. Once a client is authenticated, this frame format is the stable
//! transport boundary for video, audio and input channels.

use std::io::{self, Read, Write};

pub const MAGIC: [u8; 4] = *b"MDES";
pub const VERSION: u16 = 1;
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum FrameKind {
    Hello = 1,
    Authenticate = 2,
    DesktopList = 3,
    SelectDesktop = 4,
    VideoConfig = 10,
    VideoData = 11,
    AudioConfig = 20,
    AudioData = 21,
    Input = 30,
    Ping = 40,
    Error = 255,
}

impl TryFrom<u16> for FrameKind {
    type Error = io::Error;
    fn try_from(value: u16) -> Result<Self, io::Error> {
        use FrameKind::*;
        match value {
            1 => Ok(Hello),
            2 => Ok(Authenticate),
            3 => Ok(DesktopList),
            4 => Ok(SelectDesktop),
            10 => Ok(VideoConfig),
            11 => Ok(VideoData),
            20 => Ok(AudioConfig),
            21 => Ok(AudioData),
            30 => Ok(Input),
            40 => Ok(Ping),
            255 => Ok(Error),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unknown frame type",
            )),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct Frame {
    pub kind: FrameKind,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn write_to(&self, mut writer: impl Write) -> io::Result<()> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame exceeds payload limit",
            ));
        }
        writer.write_all(&MAGIC)?;
        writer.write_all(&VERSION.to_be_bytes())?;
        writer.write_all(&(self.kind as u16).to_be_bytes())?;
        writer.write_all(&(self.payload.len() as u32).to_be_bytes())?;
        writer.write_all(&self.payload)
    }

    pub fn read_from(mut reader: impl Read) -> io::Result<Self> {
        let mut fixed = [0_u8; 12];
        reader.read_exact(&mut fixed)?;
        if fixed[0..4] != MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid frame magic",
            ));
        }
        let version = u16::from_be_bytes([fixed[4], fixed[5]]);
        if version != VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported protocol version",
            ));
        }
        let kind = FrameKind::try_from(u16::from_be_bytes([fixed[6], fixed[7]]))?;
        let length = u32::from_be_bytes([fixed[8], fixed[9], fixed[10], fixed[11]]) as usize;
        if length > MAX_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "declared payload exceeds limit",
            ));
        }
        let mut payload = vec![0; length];
        reader.read_exact(&mut payload)?;
        Ok(Self { kind, payload })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trip() {
        let source = Frame {
            kind: FrameKind::Ping,
            payload: b"hello".to_vec(),
        };
        let mut bytes = Vec::new();
        source.write_to(&mut bytes).unwrap();
        assert_eq!(Frame::read_from(bytes.as_slice()).unwrap(), source);
    }

    #[test]
    fn rejects_unknown_frame_kind() {
        let bytes = [b'M', b'D', b'E', b'S', 0, 1, 0, 99, 0, 0, 0, 0];
        assert!(Frame::read_from(bytes.as_slice()).is_err());
    }
}
