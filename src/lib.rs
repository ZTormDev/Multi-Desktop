//! Shared, platform-neutral protocol primitives for Multi-Desktop clients and hosts.

pub mod control {
    use std::io::{self, BufRead, Read};

    pub const BANNER: &str = "MULTIDESKTOP/1";
    pub const AUTH_PROMPT: &str = "AUTH <token>";
    pub const MAX_LINE_BYTES: u64 = 4096;

    /// Reads exactly one bounded UTF-8 line from the control protocol.
    pub fn read_line(reader: &mut impl BufRead, line: &mut String) -> io::Result<usize> {
        let mut bytes = Vec::new();
        let count = reader
            .by_ref()
            .take(MAX_LINE_BYTES + 1)
            .read_until(b'\n', &mut bytes)?;
        if count as u64 > MAX_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "protocol line exceeds 4096 bytes",
            ));
        }
        *line = String::from_utf8(bytes).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "protocol line is not UTF-8")
        })?;
        Ok(count)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Cursor;

        #[test]
        fn reads_a_bounded_utf8_line() {
            let mut reader = Cursor::new(b"PING\n".to_vec());
            let mut line = String::new();
            assert_eq!(read_line(&mut reader, &mut line).unwrap(), 5);
            assert_eq!(line, "PING\n");
        }

        #[test]
        fn rejects_an_oversized_line() {
            let mut reader = Cursor::new(vec![b'x'; MAX_LINE_BYTES as usize + 1]);
            let mut line = String::new();
            assert!(read_line(&mut reader, &mut line).is_err());
        }
    }
}

pub mod transport;
