//! Versioned binary framing shared by the future Linux server and Windows client.
//!
//! The control-plane daemon still exposes a deliberately small text bootstrap
//! protocol. Once a client is authenticated, this frame format is the stable
//! transport boundary for video, audio and input channels.

use std::io::{self, Read, Write};

pub const MAGIC: [u8; 4] = *b"MDES";
pub const VERSION: u16 = 1;
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
pub const MAX_EVDEV_CODE: u16 = 0x02ff;
pub const MIN_POINTER_BUTTON: u16 = 0x0110;
pub const MAX_POINTER_BUTTON: u16 = 0x0117;

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

/// Input payload carried inside a [`FrameKind::Input`] frame. Key and button
/// values are Linux evdev codes; pointer and scroll deltas are signed logical
/// units. The compact fixed-width forms make partial/oversized packets easy to
/// reject before they reach the session input agent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEvent {
    Key { code: u16, pressed: bool },
    PointerMotion { x: i16, y: i16 },
    PointerButton { code: u16, pressed: bool },
    Scroll { x: i16, y: i16 },
    Reset,
}

impl InputEvent {
    pub const MAX_ENCODED_LEN: usize = 5;

    pub fn encode(self) -> Vec<u8> {
        match self {
            Self::Key { code, pressed } => {
                let [high, low] = code.to_be_bytes();
                vec![1, u8::from(pressed), high, low]
            }
            Self::PointerMotion { x, y } => encode_pair(2, x, y),
            Self::PointerButton { code, pressed } => {
                let [high, low] = code.to_be_bytes();
                vec![3, u8::from(pressed), high, low]
            }
            Self::Scroll { x, y } => encode_pair(4, x, y),
            Self::Reset => vec![5],
        }
    }

    pub fn decode(payload: &[u8]) -> io::Result<Self> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid input event");
        match payload {
            [1, pressed @ 0..=1, high, low] => {
                let code = u16::from_be_bytes([*high, *low]);
                if code == 0 || code > MAX_EVDEV_CODE {
                    return Err(invalid());
                }
                Ok(Self::Key {
                    code,
                    pressed: *pressed == 1,
                })
            }
            [2, x_high, x_low, y_high, y_low] => Ok(Self::PointerMotion {
                x: i16::from_be_bytes([*x_high, *x_low]),
                y: i16::from_be_bytes([*y_high, *y_low]),
            }),
            [3, pressed @ 0..=1, high, low] => {
                let code = u16::from_be_bytes([*high, *low]);
                if !(MIN_POINTER_BUTTON..=MAX_POINTER_BUTTON).contains(&code) {
                    return Err(invalid());
                }
                Ok(Self::PointerButton {
                    code,
                    pressed: *pressed == 1,
                })
            }
            [4, x_high, x_low, y_high, y_low] => Ok(Self::Scroll {
                x: i16::from_be_bytes([*x_high, *x_low]),
                y: i16::from_be_bytes([*y_high, *y_low]),
            }),
            [5] => Ok(Self::Reset),
            _ => Err(invalid()),
        }
    }

    pub fn encoded_len_for_tag(tag: u8) -> Option<usize> {
        match tag {
            1 | 3 => Some(4),
            2 | 4 => Some(5),
            5 => Some(1),
            _ => None,
        }
    }
}

fn encode_pair(tag: u8, x: i16, y: i16) -> Vec<u8> {
    let [x_high, x_low] = x.to_be_bytes();
    let [y_high, y_low] = y.to_be_bytes();
    vec![tag, x_high, x_low, y_high, y_low]
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
        Self::read_from_with_limit(&mut reader, MAX_PAYLOAD)
    }

    pub fn read_from_with_limit(mut reader: impl Read, max_payload: usize) -> io::Result<Self> {
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
        if length > MAX_PAYLOAD || length > max_payload {
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

    #[test]
    fn input_events_round_trip() {
        let events = [
            InputEvent::Key {
                code: 30,
                pressed: true,
            },
            InputEvent::PointerMotion { x: -12, y: 25 },
            InputEvent::PointerButton {
                code: 0x110,
                pressed: false,
            },
            InputEvent::Scroll { x: 0, y: -120 },
            InputEvent::Reset,
        ];
        for event in events {
            assert_eq!(InputEvent::decode(&event.encode()).unwrap(), event);
        }
    }

    #[test]
    fn rejects_invalid_input_codes_and_shapes() {
        assert!(InputEvent::decode(&[1, 1, 0xff, 0xff]).is_err());
        assert!(InputEvent::decode(&[3, 1, 0, 1]).is_err());
        assert!(InputEvent::decode(&[1, 2, 0, 30]).is_err());
        assert!(InputEvent::decode(&[5, 0]).is_err());
    }
}
