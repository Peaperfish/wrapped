//! A minimal protobuf wire-format reader: just enough to pull a few fields out
//! of blobs whose schema is not published. Never panics: every length and
//! offset is checked, nesting is capped, and bad bytes end the walk.

const MAX_DEPTH: usize = 8;

/// One decoded field value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value<'a> {
    Varint(u64),
    Fixed32(u32),
    Fixed64(u64),
    Bytes(&'a [u8]),
}

fn varint(b: &[u8], pos: &mut usize) -> Option<u64> {
    let mut r: u64 = 0;
    for shift in (0..70).step_by(7) {
        let c = *b.get(*pos)?;
        *pos += 1;
        if shift < 64 {
            r |= u64::from(c & 0x7f) << shift;
        }
        if c & 0x80 == 0 {
            return Some(r);
        }
    }
    None
}

/// Read the fields of one message in order, calling `f(field number, value)`.
/// Returns false when the bytes stop being valid (the fields before that point
/// were still delivered).
pub fn fields<'a>(b: &'a [u8], mut f: impl FnMut(u32, Value<'a>)) -> bool {
    let mut pos = 0usize;
    while pos < b.len() {
        let Some(key) = varint(b, &mut pos) else {
            return false;
        };
        let num = key >> 3;
        if num == 0 || num > u64::from(u32::MAX) {
            return false;
        }
        let num = num as u32;
        let v = match key & 7 {
            0 => match varint(b, &mut pos) {
                Some(v) => Value::Varint(v),
                None => return false,
            },
            1 => match b.get(pos..pos.saturating_add(8)) {
                Some(s) => {
                    pos += 8;
                    Value::Fixed64(u64::from_le_bytes(s.try_into().unwrap_or_default()))
                }
                None => return false,
            },
            5 => match b.get(pos..pos.saturating_add(4)) {
                Some(s) => {
                    pos += 4;
                    Value::Fixed32(u32::from_le_bytes(s.try_into().unwrap_or_default()))
                }
                None => return false,
            },
            2 => {
                let Some(n) = varint(b, &mut pos) else {
                    return false;
                };
                let end = usize::try_from(n).ok().and_then(|n| pos.checked_add(n));
                match end.and_then(|e| b.get(pos..e)) {
                    Some(s) => {
                        pos += s.len();
                        Value::Bytes(s)
                    }
                    None => return false,
                }
            }
            _ => return false, // groups and unknown wire types
        };
        f(num, v);
    }
    true
}

/// The first value at a field path (`&[1, 19]` = field 19 of the message in
/// field 1). Only length-delimited fields are entered.
pub fn find<'a>(b: &'a [u8], path: &[u32]) -> Option<Value<'a>> {
    find_at(b, path, 0)
}

fn find_at<'a>(b: &'a [u8], path: &[u32], depth: usize) -> Option<Value<'a>> {
    let (first, rest) = path.split_first()?;
    if depth >= MAX_DEPTH {
        return None;
    }
    let mut hit = None;
    fields(b, |n, v| {
        if hit.is_some() || n != *first {
            return;
        }
        hit = if rest.is_empty() {
            Some(v)
        } else if let Value::Bytes(inner) = v {
            find_at(inner, rest, depth + 1)
        } else {
            None
        };
    });
    hit
}

pub fn find_varint(b: &[u8], path: &[u32]) -> Option<u64> {
    match find(b, path)? {
        Value::Varint(v) => Some(v),
        _ => None,
    }
}

pub fn find_str<'a>(b: &'a [u8], path: &[u32]) -> Option<&'a str> {
    match find(b, path)? {
        Value::Bytes(s) => std::str::from_utf8(s).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vi(mut v: u64) -> Vec<u8> {
        let mut o = Vec::new();
        loop {
            let c = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                o.push(c);
                return o;
            }
            o.push(c | 0x80);
        }
    }
    fn key(n: u32, w: u8) -> Vec<u8> {
        vi((u64::from(n) << 3) | u64::from(w))
    }
    fn var(n: u32, v: u64) -> Vec<u8> {
        [key(n, 0), vi(v)].concat()
    }
    fn len(n: u32, d: &[u8]) -> Vec<u8> {
        [key(n, 2), vi(d.len() as u64), d.to_vec()].concat()
    }

    #[test]
    fn reads_every_wire_type_and_nested_messages() {
        let inner = [var(2, 300), len(19, b"claude-opus-5-5-medium")].concat();
        let msg = [
            var(1, 7),
            [key(2, 1), 9u64.to_le_bytes().to_vec()].concat(),
            [key(3, 5), 5u32.to_le_bytes().to_vec()].concat(),
            len(4, &inner),
        ]
        .concat();
        let mut got = Vec::new();
        assert!(fields(&msg, |n, v| got.push((n, v))));
        assert_eq!(got.len(), 4);
        assert_eq!(got[0], (1, Value::Varint(7)));
        assert_eq!(got[1], (2, Value::Fixed64(9)));
        assert_eq!(got[2], (3, Value::Fixed32(5)));
        assert_eq!(find_varint(&msg, &[4, 2]), Some(300));
        assert_eq!(find_str(&msg, &[4, 19]), Some("claude-opus-5-5-medium"));
        assert_eq!(find_str(&msg, &[4, 20]), None);
        assert_eq!(find_str(&msg, &[1, 19]), None); // a varint is not entered
    }

    #[test]
    fn truncated_and_garbage_input_never_panics() {
        let msg = [var(1, 300), len(2, b"hello world"), var(3, u64::MAX)].concat();
        for cut in 0..msg.len() {
            let _ = fields(&msg[..cut], |_, _| {});
            let _ = find(&msg[..cut], &[2, 1]);
        }
        // Length bigger than the data, and than usize.
        assert!(!fields(
            &[key(1, 2), vi(1000), vec![1, 2]].concat(),
            |_, _| {}
        ));
        assert!(!fields(&[key(1, 2), vi(u64::MAX)].concat(), |_, _| {}));
        // Varint that never ends, field 0, groups, wire types 6 and 7.
        assert!(!fields(&[0xff; 20], |_, _| {}));
        assert!(!fields(&[0x00, 0x01], |_, _| {}));
        assert!(!fields(&key(1, 3), |_, _| {}));
        assert!(!fields(&key(1, 6), |_, _| {}));
        // Fixed-width fields cut short.
        assert!(!fields(&[key(1, 1), vec![1, 2, 3]].concat(), |_, _| {}));
        assert!(!fields(&[key(1, 5), vec![1]].concat(), |_, _| {}));
        // Every byte pattern of length 1..=3 is safe.
        for a in 0..=255u8 {
            for b in [0u8, 1, 2, 0x80, 0xff] {
                let _ = fields(&[a, b, a], |_, _| {});
                let _ = find(&[a, b, a, b], &[1, 1]);
            }
        }
    }

    #[test]
    fn deep_nesting_stops_at_the_cap() {
        let mut m = len(1, b"x");
        for _ in 0..50 {
            m = len(1, &m);
        }
        assert_eq!(find(&m, &[1; 40]), None);
        assert!(find(&m, &[1; 3]).is_some());
    }

    #[test]
    fn valid_fields_before_the_damage_are_still_delivered() {
        let msg = [var(1, 5), vec![0xff]].concat();
        let mut n = 0;
        assert!(!fields(&msg, |_, _| n += 1));
        assert_eq!(n, 1);
    }
}
