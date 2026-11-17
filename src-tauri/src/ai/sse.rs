//! Incremental Server-Sent Events parser: bytes in, `data:` payloads out.

#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    data: String,
    has_data: bool,
}

impl SseParser {
    /// Feeds a chunk and returns the payload of every event it completed.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if self.has_data {
                    events.push(std::mem::take(&mut self.data));
                    self.has_data = false;
                }
            } else if let Some(value) = line.strip_prefix("data:") {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value.strip_prefix(' ').unwrap_or(value));
                self.has_data = true;
            }
            // `event:`, `id:`, `retry:` and comments carry nothing we need:
            // every provider repeats the event type inside the JSON.
        }
        events
    }

    /// The payload of an event the server did not terminate with a blank line.
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buffer);
        if !rest.is_empty() {
            let mut line = rest;
            line.push(b'\n');
            let mut events = self.feed(&line);
            if let Some(event) = events.pop() {
                return Some(event);
            }
        }
        if self.has_data {
            self.has_data = false;
            return Some(std::mem::take(&mut self.data));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_split_across_chunks_and_crlf() {
        let mut p = SseParser::default();
        assert!(p.feed(b"event: x\r\ndata: {\"a\"").is_empty());
        assert_eq!(p.feed(b":1}\r\n\r\ndata: two\n\n"), ["{\"a\":1}", "two"]);
    }

    #[test]
    fn multi_line_data_and_comments() {
        let mut p = SseParser::default();
        assert_eq!(p.feed(b": ping\n\ndata: a\ndata: b\n\n"), ["a\nb"]);
    }

    #[test]
    fn utf8_split_in_the_middle_of_a_character() {
        let bytes = "data: héllo\n\n".as_bytes();
        let mut p = SseParser::default();
        assert!(p.feed(&bytes[..8]).is_empty());
        assert_eq!(p.feed(&bytes[8..]), ["héllo"]);
    }

    #[test]
    fn unterminated_last_event_is_still_delivered() {
        let mut p = SseParser::default();
        assert!(p.feed(b"data: tail").is_empty());
        assert_eq!(p.finish().as_deref(), Some("tail"));
        assert_eq!(p.finish(), None);
    }
}
