//! JavaScript-compatibility helpers. The Node implementation is the reference, so every place where its output
//! depends on JS semantics (UTF-16 string lengths, `trim`, number formatting, `JSON.stringify`, `Date`) goes through
//! here. Known, documented deviation: a JS slice that cuts a surrogate pair leaves a lone surrogate; Rust strings
//! cannot hold one, so it becomes U+FFFD (which is also what Node writes when that string reaches HTTP as UTF-8).
use serde_json::Value;
use std::fmt::Write as _;

// ---------------------------------------------------------------- strings (UTF-16 semantics)

/// JS `s.length`.
pub fn len16(s: &str) -> usize {
    if s.is_ascii() { s.len() } else { s.chars().map(char::len_utf16).sum() }
}

/// JS `s.slice(start, end)` with non-negative, already-clamped UTF-16 indices.
pub fn slice16(s: &str, start: usize, end: usize) -> String {
    if s.is_ascii() {
        let end = end.min(s.len());
        let start = start.min(end);
        return s[start..end].to_string();
    }
    let mut out = String::new();
    let mut pos = 0usize;
    for c in s.chars() {
        let w = c.len_utf16();
        let (a, b) = (pos, pos + w);
        pos = b;
        if b <= start { continue; }
        if a >= end { break; }
        if a >= start && b <= end { out.push(c); } else { out.push('\u{FFFD}'); } // half of a surrogate pair
    }
    out
}

/// JS `s.slice(0, n)`.
pub fn head16(s: &str, n: usize) -> String { slice16(s, 0, n) }

/// JS `s.slice(-n)` for n > 0.
pub fn tail16(s: &str, n: usize) -> String {
    let l = len16(s);
    slice16(s, l.saturating_sub(n), l)
}

/// The JS WhiteSpace + LineTerminator set (`trim`, `\s`).
pub fn is_ws(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}'
        | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
}

/// The JS `\s` class for use inside a regex character class.
pub const RE_S: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

pub fn trim(s: &str) -> &str { s.trim_matches(is_ws) }

/// JS `padEnd(n)` with spaces.
pub fn pad_end(s: &str, n: usize) -> String {
    let l = len16(s);
    if l >= n { s.to_string() } else { format!("{s}{}", " ".repeat(n - l)) }
}

// ---------------------------------------------------------------- values

pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => { let f = n.as_f64().unwrap_or(0.0); f != 0.0 && !f.is_nan() }
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// Property access `v.key` (undefined unless `v` is an object).
pub fn prop<'a>(v: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    match v { Some(Value::Object(m)) => m.get(key), _ => None }
}

/// JS `a || b`.
pub fn or<'a>(a: Option<&'a Value>, b: Option<&'a Value>) -> Option<&'a Value> { if truthy(a) { a } else { b } }

/// JS `String(v)`; `None` is `undefined`.
pub fn string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(v) => value_string(v),
    }
}

fn value_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(|x| if x.is_null() { String::new() } else { value_string(x) }).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// JS `String(v ?? '')`.
pub fn string_or_empty(v: Option<&Value>) -> String {
    match v { None | Some(Value::Null) => String::new(), Some(v) => value_string(v) }
}

/// JS `Number(v)`.
pub fn number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => if *b { 1.0 } else { 0.0 },
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => str_to_number(s),
        Some(Value::Array(a)) => match a.len() { 0 => 0.0, 1 => number(Some(&a[0])), _ => f64::NAN },
        Some(Value::Object(_)) => f64::NAN,
    }
}

/// JS `Number(string)`.
pub fn str_to_number(s: &str) -> f64 {
    let t = trim(s);
    if t.is_empty() { return 0.0; }
    let radix = |p: &str, r: u32| -> f64 {
        if p.is_empty() || !p.chars().all(|c| c.is_digit(r)) { return f64::NAN; }
        p.chars().fold(0f64, |acc, c| acc * r as f64 + c.to_digit(r).unwrap() as f64)
    };
    let lower = t.to_ascii_lowercase();
    if let Some(p) = lower.strip_prefix("0x") { return radix(p, 16); }
    if let Some(p) = lower.strip_prefix("0o") { return radix(p, 8); }
    if let Some(p) = lower.strip_prefix("0b") { return radix(p, 2); }
    match t { "Infinity" | "+Infinity" => return f64::INFINITY, "-Infinity" => return f64::NEG_INFINITY, _ => {} }
    // StrDecimalLiteral: [+-] (digits [. digits] | . digits) [e[+-]digits]
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') { i += 1; }
    let ds = i;
    while i < b.len() && b[i].is_ascii_digit() { i += 1; }
    let mut digits = i - ds;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let fs = i;
        while i < b.len() && b[i].is_ascii_digit() { i += 1; }
        digits += i - fs;
    }
    if digits == 0 { return f64::NAN; }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') { i += 1; }
        let es = i;
        while i < b.len() && b[i].is_ascii_digit() { i += 1; }
        if i == es { return f64::NAN; }
    }
    if i != b.len() { return f64::NAN; }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// JS `Number.prototype.toString()` (shortest round-trip, JS exponent rules).
pub fn num(x: f64) -> String {
    if x.is_nan() { return "NaN".into(); }
    if x.is_infinite() { return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() }; }
    if x == 0.0 { return "0".into(); }
    if x.fract() == 0.0 && x.abs() < 1e21 { return format!("{}", x as i128); }
    let neg = x < 0.0;
    let e = format!("{:e}", x.abs());
    let (mant, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp + 1;
    let mut s = String::new();
    if neg { s.push('-'); }
    if k <= n && n <= 21 {
        s.push_str(&digits);
        for _ in 0..(n - k) { s.push('0'); }
    } else if 0 < n && n <= 21 {
        s.push_str(&digits[..n as usize]);
        s.push('.');
        s.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        s.push_str("0.");
        for _ in 0..(-n) { s.push('0'); }
        s.push_str(&digits);
    } else {
        s.push_str(&digits[..1]);
        if k > 1 { s.push('.'); s.push_str(&digits[1..]); }
        let _ = write!(s, "e{}{}", if n - 1 >= 0 { '+' } else { '-' }, (n - 1).abs());
    }
    s
}

/// `${x}` for an optional number (null prints "null").
pub fn num_opt(x: Option<f64>) -> String { x.map(num).unwrap_or_else(|| "null".into()) }

/// JS `x.toFixed(1)` for finite, non-negative-or-negative x below 1e21 (round half up on the exact binary value).
pub fn to_fixed1(x: f64) -> String {
    let neg = x < 0.0;
    let exact = format!("{:.80}", x.abs()); // exact decimal expansion for any f64 we format here
    let (ip, fp) = exact.split_once('.').unwrap();
    let mut int: Vec<u8> = ip.bytes().collect();
    let mut d1 = fp.as_bytes()[0] - b'0';
    let rest = &fp[1..];
    let round_up = rest.as_bytes()[0] >= b'5'; // >= .x5 (ties go up, like toFixed's "larger n")
    if round_up {
        if d1 == 9 {
            d1 = 0;
            let mut i = int.len();
            loop {
                if i == 0 { int.insert(0, b'1'); break; }
                i -= 1;
                if int[i] == b'9' { int[i] = b'0'; } else { int[i] += 1; break; }
            }
        } else { d1 += 1; }
    }
    let body = format!("{}.{}", String::from_utf8(int).unwrap(), d1);
    if neg && body != "0.0" { format!("-{body}") } else { body }
}

/// JS `Math.round` for the non-negative values we use.
pub fn round(x: f64) -> f64 { (x + 0.5).floor() }

// ---------------------------------------------------------------- JSON.stringify

pub fn json(v: &Value) -> String { let mut s = String::new(); write_json(v, &mut s, None, 0); s }
pub fn json_pretty(v: &Value) -> String { let mut s = String::new(); write_json(v, &mut s, Some(2), 0); s }

pub fn json_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => { let _ = write!(out, "\\u{:04x}", c as u32); }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn is_index_key(k: &str) -> bool {
    if k.is_empty() || k.len() > 10 || !k.bytes().all(|b| b.is_ascii_digit()) { return false; }
    if k.len() > 1 && k.starts_with('0') { return false; }
    k.parse::<u64>().map(|n| n < 4_294_967_295).unwrap_or(false)
}

/// Keys in JS enumeration order: array-index keys ascending first, then the rest in insertion order.
pub fn js_keys(m: &serde_json::Map<String, Value>) -> Vec<&String> {
    if !m.keys().any(|k| is_index_key(k)) { return m.keys().collect(); }
    let mut idx: Vec<&String> = m.keys().filter(|k| is_index_key(k)).collect();
    idx.sort_by_key(|k| k.parse::<u64>().unwrap());
    idx.extend(m.keys().filter(|k| !is_index_key(k)));
    idx
}

fn write_json(v: &Value, out: &mut String, indent: Option<usize>, level: usize) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            let f = n.as_f64().unwrap_or(f64::NAN);
            if f.is_finite() { out.push_str(&num(f)); } else { out.push_str("null"); }
        }
        Value::String(s) => json_str(s, out),
        Value::Array(a) => {
            if a.is_empty() { out.push_str("[]"); return; }
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 { out.push(','); }
                newline(out, indent, level + 1);
                write_json(x, out, indent, level + 1);
            }
            newline(out, indent, level);
            out.push(']');
        }
        Value::Object(m) => {
            if m.is_empty() { out.push_str("{}"); return; }
            out.push('{');
            for (i, k) in js_keys(m).into_iter().enumerate() {
                if i > 0 { out.push(','); }
                newline(out, indent, level + 1);
                json_str(k, out);
                out.push(':');
                if indent.is_some() { out.push(' '); }
                write_json(&m[k.as_str()], out, indent, level + 1);
            }
            newline(out, indent, level);
            out.push('}');
        }
    }
}

fn newline(out: &mut String, indent: Option<usize>, level: usize) {
    if let Some(n) = indent { out.push('\n'); for _ in 0..n * level { out.push(' '); } }
}

/// Parse one JSONL line the way `JSON.parse` would. serde rejects lone-surrogate escapes that JS accepts; on that
/// failure the escapes are replaced with U+FFFD and the line is parsed again.
pub fn parse_line(line: &str) -> Option<Value> {
    match serde_json::from_str::<Value>(line) {
        Ok(v) => Some(v),
        Err(_) if line.contains("\\u") || line.contains("\\U") => serde_json::from_str(&fix_lone_surrogates(line)).ok(),
        Err(_) => None,
    }
}

fn fix_lone_surrogates(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let hex = |i: usize| -> Option<u32> {
        if i + 6 <= b.len() && b[i] == b'\\' && b[i + 1] == b'u' { u32::from_str_radix(std::str::from_utf8(&b[i + 2..i + 6]).ok()?, 16).ok() } else { None }
    };
    let mut last = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            if let Some(u) = hex(i) {
                if (0xD800..0xDC00).contains(&u) {
                    if let Some(l) = hex(i + 6) { if (0xDC00..0xE000).contains(&l) { i += 12; continue; } }
                    out.push_str(&s[last..i]); out.push_str("\\ufffd"); i += 6; last = i; continue;
                }
                if (0xDC00..0xE000).contains(&u) { out.push_str(&s[last..i]); out.push_str("\\ufffd"); i += 6; last = i; continue; }
                i += 6; continue;
            }
            i += 2; continue;
        }
        i += 1;
    }
    out.push_str(&s[last..]);
    out
}

// ---------------------------------------------------------------- Date

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `new Date(ms).toISOString()`.
pub fn iso(ms: f64) -> String {
    let ms = ms.trunc() as i64;
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, mo, d) = civil_from_days(days);
    let ys = if (0..=9999).contains(&y) { format!("{y:04}") } else if y < 0 { format!("-{:06}", -y) } else { format!("+{y:06}") };
    format!("{ys}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", rem / 3_600_000, rem / 60_000 % 60, rem / 1000 % 60, rem % 1000)
}

/// `Date.parse` for ISO-8601 strings (what Claude Code writes). Other formats return None.
pub fn date_parse(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize, n: usize| -> Option<i64> {
        if *i + n > b.len() || !b[*i..*i + n].iter().all(u8::is_ascii_digit) { return None; }
        let v = std::str::from_utf8(&b[*i..*i + n]).ok()?.parse().ok()?;
        *i += n;
        Some(v)
    };
    let year = if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        let neg = b[i] == b'-'; i += 1;
        let y = digits(&mut i, 6)?;
        if neg && y == 0 { return None; }
        if neg { -y } else { y }
    } else { digits(&mut i, 4)? };
    let (mut mo, mut d) = (1, 1);
    let (mut h, mut mi, mut sec, mut msv) = (0, 0, 0, 0);
    let mut date_only = true;
    if i < b.len() && b[i] == b'-' {
        i += 1; mo = digits(&mut i, 2)?;
        if i < b.len() && b[i] == b'-' { i += 1; d = digits(&mut i, 2)?; }
    }
    let mut offset_min: Option<i64> = None;
    if i < b.len() && (b[i] == b'T' || b[i] == b't') {
        date_only = false;
        i += 1; h = digits(&mut i, 2)?;
        if i >= b.len() || b[i] != b':' { return None; }
        i += 1; mi = digits(&mut i, 2)?;
        if i < b.len() && b[i] == b':' {
            i += 1; sec = digits(&mut i, 2)?;
            if i < b.len() && (b[i] == b'.' || b[i] == b',') {
                i += 1;
                let st = i;
                while i < b.len() && b[i].is_ascii_digit() { i += 1; }
                if i == st { return None; }
                let frac = &s[st..i];
                let f3: String = frac.chars().chain("00".chars()).take(3).collect();
                msv = f3.parse().ok()?;
            }
        }
        if i < b.len() && (b[i] == b'Z' || b[i] == b'z') { i += 1; offset_min = Some(0); }
        else if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            let sign = if b[i] == b'-' { -1 } else { 1 }; i += 1;
            let oh = digits(&mut i, 2)?;
            if i < b.len() && b[i] == b':' { i += 1; }
            let om = digits(&mut i, 2)?;
            offset_min = Some(sign * (oh * 60 + om));
        }
    }
    if i != b.len() { return None; }
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 24 || mi > 59 || sec > 59 { return None; }
    if h == 24 && (mi != 0 || sec != 0 || msv != 0) { return None; }
    let off = match offset_min { Some(o) => o, None if date_only => 0, None => return None /* local time: unsupported */ };
    let days = days_from_civil(year, mo, d);
    Some((days * 86_400_000 + h * 3_600_000 + mi * 60_000 + sec * 1000 + msv - off * 60_000) as f64)
}

// ---------------------------------------------------------------- misc

/// JS `encodeURIComponent`.
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) { out.push(b as char); } else { let _ = write!(out, "%{b:02X}"); }
    }
    out
}

/// JS `decodeURIComponent`; Err("URI malformed") like V8.
pub fn decode_uri_component(s: &str) -> Result<String, String> {
    if !s.contains('%') { return Ok(s.to_string()); }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            match h { Some(v) => { out.push(v); i += 3; } None => return Err("URI malformed".into()) }
        } else { out.push(b[i]); i += 1; }
    }
    String::from_utf8(out).map_err(|_| "URI malformed".into())
}

/// Node `Buffer.from(s, 'base64')`: lenient (url-safe alphabet, whitespace and junk skipped, stops at padding).
pub fn base64_decode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); acc &= (1 << bits) - 1; }
    }
    out
}

pub fn esc_html(s: &str) -> String { let mut o = String::with_capacity(s.len() + 16); esc_html_into(&mut o, s); o }

/// Appends `s` HTML-escaped (& < > " ') to `o`, copying unescaped runs in one go.
pub fn esc_html_into(o: &mut String, s: &str) {
    let mut last = 0;
    for (i, &c) in s.as_bytes().iter().enumerate() {
        let r = match c { b'&' => "&amp;", b'<' => "&lt;", b'>' => "&gt;", b'"' => "&quot;", b'\'' => "&#39;", _ => continue };
        o.push_str(&s[last..i]);
        o.push_str(r);
        last = i + 1;
    }
    o.push_str(&s[last..]);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(1731430000123.25), "1731430000123.25");
        assert_eq!(num(0.1), "0.1");
        assert_eq!(num(1e21), "1e+21");
        assert_eq!(num(1.5e-7), "1.5e-7");
        assert_eq!(num(-2.5), "-2.5");
        assert_eq!(num(0.000001), "0.000001");
        assert_eq!(to_fixed1(1.25), "1.3");
        assert_eq!(to_fixed1(1.35), "1.4");
        assert_eq!(to_fixed1(9.96), "10.0");
        assert_eq!(to_fixed1(3.0), "3.0");
    }
    #[test]
    fn dates() {
        let t = date_parse("2026-01-01T10:00:00.000Z").unwrap();
        assert_eq!(iso(t), "2026-01-01T10:00:00.000Z");
        assert_eq!(date_parse("2026-01-01T10:00:00.5Z").unwrap() - t, 500.0);
        assert_eq!(date_parse("2026-01-01T11:00:00+01:00").unwrap(), t);
        assert_eq!(iso(0.0), "1970-01-01T00:00:00.000Z");
    }
    #[test]
    fn strings() {
        assert_eq!(len16("a😀"), 3);
        assert_eq!(slice16("a😀b", 0, 2), "a\u{FFFD}");
        assert_eq!(tail16("hello", 2), "lo");
        assert_eq!(json(&serde_json::json!({"b":1,"2":[1.0,"x\n"],"1":null})), r#"{"1":null,"2":[1,"x\n"],"b":1}"#);
    }
}
