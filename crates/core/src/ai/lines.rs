//! Streaming line reader for JSONL logs: bounded memory, byte positions.
//!
//! * Lines longer than `max` bytes are skipped without being stored (a tool
//!   result of many MB must not become a many-MB buffer).
//! * `pos` is the offset after the last *complete* line: a final line with no
//!   newline yet (the tool is still writing it) is left for the next scan.

use std::io::{self, BufRead};

pub struct Lines<R: BufRead> {
    r: R,
    buf: Vec<u8>,
    max: usize,
    /// Offset after the last complete line returned (relative to where the
    /// reader started, plus `base`).
    pos: u64,
}

pub enum Line<'a> {
    Text(&'a str),
    /// A line over the size limit: skipped, not stored.
    TooLong,
    /// A complete line that is not valid UTF-8.
    Invalid,
}

impl<R: BufRead> Lines<R> {
    /// `base`: the byte offset of the reader's first byte in the file.
    pub fn new(r: R, base: u64, max: usize) -> Self {
        Self {
            r,
            buf: Vec::new(),
            max,
            pos: base,
        }
    }

    /// Offset after the last complete line returned so far.
    pub fn pos(&self) -> u64 {
        self.pos
    }

    /// The next complete line, or `None` at the end of the data (a partial
    /// last line is not returned and not counted in `pos`).
    pub fn next_line(&mut self) -> io::Result<Option<Line<'_>>> {
        self.buf.clear();
        let mut overflow = false;
        let mut consumed: u64 = 0;
        loop {
            let avail = self.r.fill_buf()?;
            if avail.is_empty() {
                return Ok(None);
            }
            match avail.iter().position(|&b| b == b'\n') {
                Some(i) => {
                    if !overflow {
                        if self.buf.len() + i <= self.max {
                            self.buf.extend_from_slice(&avail[..i]);
                        } else {
                            overflow = true;
                        }
                    }
                    self.r.consume(i + 1);
                    self.pos += consumed + i as u64 + 1;
                    if overflow {
                        self.buf.clear();
                        return Ok(Some(Line::TooLong));
                    }
                    if self.buf.last() == Some(&b'\r') {
                        self.buf.pop();
                    }
                    return Ok(Some(match std::str::from_utf8(&self.buf) {
                        Ok(s) => Line::Text(s),
                        Err(_) => Line::Invalid,
                    }));
                }
                None => {
                    let n = avail.len();
                    if !overflow {
                        if self.buf.len() + n <= self.max {
                            self.buf.extend_from_slice(avail);
                        } else {
                            overflow = true;
                            self.buf.clear();
                        }
                    }
                    consumed += n as u64;
                    self.r.consume(n);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(data: &[u8], base: u64, max: usize) -> (Vec<String>, u64) {
        let mut l = Lines::new(io::BufReader::with_capacity(4, data), base, max);
        let mut out = Vec::new();
        while let Some(line) = l.next_line().unwrap() {
            out.push(match line {
                Line::Text(t) => t.to_string(),
                Line::TooLong => "<long>".into(),
                Line::Invalid => "<bad>".into(),
            });
        }
        (out, l.pos())
    }

    #[test]
    fn complete_lines_and_positions() {
        let (v, pos) = collect(b"ab\ncd\r\n\nef\n", 100, 64);
        assert_eq!(v, ["ab", "cd", "", "ef"]);
        assert_eq!(pos, 100 + 11);
    }

    #[test]
    fn partial_last_line_is_left_for_next_time() {
        let (v, pos) = collect(b"one\ntwo\nthr", 0, 64);
        assert_eq!(v, ["one", "two"]);
        assert_eq!(pos, 8);
    }

    #[test]
    fn long_lines_are_skipped_not_stored() {
        let mut data = vec![b'x'; 50];
        data.extend_from_slice(b"\nok\n");
        let (v, pos) = collect(&data, 0, 10);
        assert_eq!(v, ["<long>", "ok"]);
        assert_eq!(pos, 54);
    }

    #[test]
    fn invalid_utf8_is_reported_and_skipped() {
        let (v, _) = collect(b"a\n\xff\xfe\nb\n", 0, 64);
        assert_eq!(v, ["a", "<bad>", "b"]);
    }
}
