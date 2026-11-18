use bytes::{Buf, BytesMut};

use super::ProviderError;

// A model event is one small delta or terminal record. Bound unknown fields
// and incomplete frames before passing bytes to the JSON parser.
const MAX_FRAME_BYTES: usize = 1024 * 1024;

pub(super) struct Frame {
    pub(super) data: Vec<u8>,
    pub(super) is_error: bool,
}

#[derive(Default)]
pub(super) struct Decoder {
    pending: BytesMut,
    data: Vec<u8>,
    frame_bytes: usize,
    skip_lf: bool,
    read_line: bool,
    is_error: bool,
}

impl Decoder {
    pub(super) fn push(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
    }

    pub(super) fn next(&mut self) -> Result<Option<Frame>, ProviderError> {
        loop {
            if self.skip_lf && !self.pending.is_empty() {
                if self.pending[0] == b'\n' {
                    self.pending.advance(1);
                }
                self.skip_lf = false;
            }
            let Some(end) = self
                .pending
                .iter()
                .position(|byte| matches!(byte, b'\n' | b'\r'))
            else {
                if self.frame_bytes + self.pending.len() > MAX_FRAME_BYTES {
                    return Err(ProviderError::InvalidProtocol);
                }
                return Ok(None);
            };
            self.frame_bytes += end + 1;
            if self.frame_bytes > MAX_FRAME_BYTES {
                return Err(ProviderError::InvalidProtocol);
            }
            let line = self.pending.split_to(end + 1);
            self.skip_lf = line[end] == b'\r';
            let mut line = &line[..end];
            if !self.read_line {
                self.read_line = true;
                line = line.strip_prefix(b"\xef\xbb\xbf").unwrap_or(line);
            }
            if line.is_empty() {
                self.frame_bytes = 0;
                let is_error = std::mem::take(&mut self.is_error);
                if is_error || !self.data.is_empty() {
                    // SSE concatenates repeated data fields, discarding the
                    // last separator when delivering the event.
                    self.data.pop();
                    return Ok(Some(Frame {
                        data: std::mem::take(&mut self.data),
                        is_error,
                    }));
                }
            } else if let Some(value) = line.strip_prefix(b"data:") {
                self.data.extend_from_slice(strip_space(value));
                self.data.push(b'\n');
            } else if let Some(value) = line.strip_prefix(b"event:") {
                self.is_error = strip_space(value) == b"error";
            }
        }
    }
}

fn strip_space(value: &[u8]) -> &[u8] {
    value.strip_prefix(b" ").unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t61_sse_handles_utf8_crlf_and_multiline_frames_across_chunks() {
        let mut decoder = Decoder::default();
        for bytes in [
            b"\xef\xbb\xbf: keepalive\r".as_slice(),
            b"\n\r\ndata: first\r\ndata: \xe4",
            b"\xb8",
        ] {
            decoder.push(bytes);
            assert!(decoder.next().unwrap().is_none());
        }

        decoder.push(b"\xad\r");
        assert!(decoder.next().unwrap().is_none());
        decoder.push(b"\n\r\n");
        let frame = decoder.next().unwrap().unwrap();
        assert!(!frame.is_error);
        assert_eq!(frame.data, "first\n中".as_bytes());
        assert!(decoder.next().unwrap().is_none());
    }
}
