//! Renders PostgreSQL values as the text `psql` would show.
//!
//! `tokio-postgres` always asks for results in PostgreSQL's binary format (that
//! is what makes a protocol-level cursor possible), so each type's wire format
//! is decoded here. The integration tests compare this output against the
//! server's own `::text` rendering for every supported type.
//!
//! Unknown types fall back to their bytes: shown as text when they are valid
//! UTF-8 (true for most extension types), otherwise as hex.

use std::fmt::Write;
use std::net::{Ipv4Addr, Ipv6Addr};

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use tokio_postgres::types::{FromSql, Kind, Type};

use super::{hex, ColumnKind};

/// Captures a column's raw bytes whatever its type.
pub struct Raw<'a>(pub Option<&'a [u8]>);

impl<'a> FromSql<'a> for Raw<'a> {
    fn from_sql(_: &Type, raw: &'a [u8]) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(Some(raw)))
    }
    fn from_sql_null(_: &Type) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(None))
    }
    fn accepts(_: &Type) -> bool {
        true
    }
}

pub fn column_kind(ty: &Type) -> ColumnKind {
    match ty.kind() {
        Kind::Domain(inner) => return column_kind(inner),
        Kind::Array(_) | Kind::Range(_) | Kind::Multirange(_) | Kind::Composite(_) => {
            return ColumnKind::Other
        }
        Kind::Enum(_) => return ColumnKind::Text,
        _ => {}
    }
    match ty.name() {
        "int2" | "int4" | "int8" | "float4" | "float8" | "numeric" | "money" | "oid" => {
            ColumnKind::Number
        }
        "bool" => ColumnKind::Bool,
        "date" | "time" | "timetz" | "timestamp" | "timestamptz" | "interval" => {
            ColumnKind::Temporal
        }
        "bytea" => ColumnKind::Binary,
        "json" | "jsonb" => ColumnKind::Json,
        "text" | "varchar" | "bpchar" | "name" | "char" | "citext" | "uuid" | "xml" | "unknown" => {
            ColumnKind::Text
        }
        _ => ColumnKind::Other,
    }
}

/// Renders one non-NULL value.
pub fn render(ty: &Type, raw: &[u8]) -> String {
    decode(ty, raw).unwrap_or_else(|| fallback(raw))
}

fn fallback(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(text) if !text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') => {
            text.to_string()
        }
        _ => hex(raw),
    }
}

fn text(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).into_owned()
}

/// A cursor over big-endian wire data.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn i16(&mut self) -> Option<i16> {
        self.take(2).map(|b| i16::from_be_bytes([b[0], b[1]]))
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_be_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Option<i32> {
        self.take(4).map(|b| i32::from_be_bytes(b.try_into().unwrap()))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_be_bytes(b.try_into().unwrap()))
    }
    fn i64(&mut self) -> Option<i64> {
        self.take(8).map(|b| i64::from_be_bytes(b.try_into().unwrap()))
    }
    fn f32(&mut self) -> Option<f32> {
        self.take(4).map(|b| f32::from_be_bytes(b.try_into().unwrap()))
    }
    fn f64(&mut self) -> Option<f64> {
        self.take(8).map(|b| f64::from_be_bytes(b.try_into().unwrap()))
    }
    /// A length-prefixed field; `None` inside means SQL NULL.
    fn field(&mut self) -> Option<Option<&'a [u8]>> {
        let len = self.i32()?;
        if len < 0 {
            Some(None)
        } else {
            self.take(len as usize).map(Some)
        }
    }
}

fn decode(ty: &Type, raw: &[u8]) -> Option<String> {
    match ty.kind() {
        Kind::Domain(inner) => return decode(inner, raw),
        Kind::Enum(_) => return Some(text(raw)),
        Kind::Array(element) => return array(element, raw),
        Kind::Range(element) => return range(element, &mut Reader(raw)),
        Kind::Multirange(element) => return multirange(element, raw),
        Kind::Composite(fields) => {
            let types: Vec<&Type> = fields.iter().map(|f| f.type_()).collect();
            return composite(&types, raw);
        }
        _ => {}
    }

    let mut r = Reader(raw);
    Some(match ty.name() {
        "bool" => if r.u8()? != 0 { "true" } else { "false" }.to_string(),
        "int2" => r.i16()?.to_string(),
        "int4" => r.i32()?.to_string(),
        "int8" => r.i64()?.to_string(),
        "oid" | "xid" | "cid" | "regclass" | "regtype" | "regproc" | "regnamespace" | "regrole" => {
            r.u32()?.to_string()
        }
        "float4" => float(r.f32()? as f64, true),
        "float8" => float(r.f64()?, false),
        "numeric" => numeric(&mut r)?,
        "money" => money(r.i64()?),
        "text" | "varchar" | "bpchar" | "name" | "unknown" | "xml" | "json" | "citext" | "char"
        | "jsonpath" | "refcursor" => text(raw),
        "jsonb" => {
            r.u8()?; // format version
            text(r.0)
        }
        "bytea" => hex(raw),
        "uuid" => uuid(r.take(16)?),
        "date" => date(r.i32()?),
        "time" => time(r.i64()?),
        "timetz" => {
            let micros = r.i64()?;
            let west = r.i32()?;
            format!("{}{}", time(micros), zone(-west))
        }
        "timestamp" => timestamp(r.i64()?, false),
        "timestamptz" => timestamp(r.i64()?, true),
        "interval" => {
            let micros = r.i64()?;
            let days = r.i32()?;
            let months = r.i32()?;
            interval(months, days, micros)
        }
        "inet" | "cidr" => inet(&mut r)?,
        "macaddr" | "macaddr8" => raw
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":"),
        "bit" | "varbit" => {
            let bits = r.i32()? as usize;
            let mut out = String::with_capacity(bits);
            for i in 0..bits {
                let byte = *r.0.get(i / 8)?;
                out.push(if byte & (0x80 >> (i % 8)) != 0 { '1' } else { '0' });
            }
            out
        }
        "pg_lsn" => {
            let v = r.i64()? as u64;
            format!("{:X}/{:X}", v >> 32, v & 0xFFFF_FFFF)
        }
        "tid" => format!("({},{})", r.u32()?, r.u16()?),
        "point" => format!("({},{})", float(r.f64()?, false), float(r.f64()?, false)),
        "void" => String::new(),
        "hstore" => hstore(&mut r)?,
        // pgvector
        "vector" => {
            let dims = r.u16()?;
            r.u16()?;
            let values: Option<Vec<String>> =
                (0..dims).map(|_| r.f32().map(|f| float(f as f64, true))).collect();
            format!("[{}]", values?.join(","))
        }
        _ => return None,
    })
}

/// PostgreSQL prints the shortest text that round-trips, which is also what
/// Rust's formatter produces; only the special values and exponents differ.
fn float(value: f64, single: bool) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let magnitude = value.abs();
    let (low, high) = if single { (1e-4, 1e9) } else { (1e-4, 1e15) };
    if magnitude != 0.0 && (magnitude < low || magnitude >= high) {
        // Scientific notation, PostgreSQL style: 1e+20, 1.5e-07.
        let text = if single {
            format!("{:e}", value as f32)
        } else {
            format!("{value:e}")
        };
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        let exponent: i32 = exponent.parse().unwrap_or(0);
        return format!(
            "{mantissa}e{}{:02}",
            if exponent < 0 { '-' } else { '+' },
            exponent.abs()
        );
    }
    if single {
        format!("{}", value as f32)
    } else {
        format!("{value}")
    }
}

fn numeric(r: &mut Reader) -> Option<String> {
    let ndigits = r.i16()? as usize;
    let weight = r.i16()? as i32;
    let sign = r.u16()?;
    let dscale = r.u16()? as usize;
    match sign {
        0xC000 => return Some("NaN".into()),
        0xD000 => return Some("Infinity".into()),
        0xF000 => return Some("-Infinity".into()),
        _ => {}
    }
    let digits: Option<Vec<u16>> = (0..ndigits).map(|_| r.u16()).collect();
    let digits = digits?;
    let digit = |i: i32| -> u16 {
        if i >= 0 && (i as usize) < digits.len() {
            digits[i as usize]
        } else {
            0
        }
    };

    let mut out = String::new();
    if sign == 0x4000 {
        out.push('-');
    }
    // Integer part: base-10000 groups 0..=weight.
    if weight < 0 {
        out.push('0');
    } else {
        for i in 0..=weight {
            if i == 0 {
                let _ = write!(out, "{}", digit(i));
            } else {
                let _ = write!(out, "{:04}", digit(i));
            }
        }
    }
    // Fractional part, cut to the display scale.
    if dscale > 0 {
        out.push('.');
        let mut fraction = String::new();
        let mut i = weight + 1;
        while fraction.len() < dscale {
            let _ = write!(fraction, "{:04}", digit(i));
            i += 1;
        }
        out.push_str(&fraction[..dscale]);
    }
    Some(out)
}

fn money(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let units = (abs / 100).to_string();
    let mut grouped = String::new();
    for (i, c) in units.chars().enumerate() {
        if i > 0 && (units.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{sign}${grouped}.{:02}", abs % 100)
}

fn uuid(b: &[u8]) -> String {
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(2000, 1, 1).expect("valid date")
}

fn date(days: i32) -> String {
    match days {
        i32::MAX => "infinity".into(),
        i32::MIN => "-infinity".into(),
        _ => match epoch().checked_add_signed(Duration::days(days as i64)) {
            Some(d) => format_date(d),
            None => format!("{days} days from 2000-01-01"),
        },
    }
}

fn format_date(d: NaiveDate) -> String {
    use chrono::Datelike;
    if d.year() <= 0 {
        // PostgreSQL has no year zero: 0000 is 1 BC.
        format!("{:04}-{:02}-{:02} BC", 1 - d.year(), d.month(), d.day())
    } else {
        format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
    }
}

fn time(micros: i64) -> String {
    let secs = micros.div_euclid(1_000_000);
    let frac = micros.rem_euclid(1_000_000);
    let mut out = format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    push_fraction(&mut out, frac);
    out
}

fn push_fraction(out: &mut String, micros: i64) {
    if micros != 0 {
        let digits = format!("{micros:06}");
        out.push('.');
        out.push_str(digits.trim_end_matches('0'));
    }
}

/// `+00`, `-05`, `+05:30`, the way PostgreSQL prints zone offsets.
fn zone(east_seconds: i32) -> String {
    let sign = if east_seconds < 0 { '-' } else { '+' };
    let abs = east_seconds.abs();
    let (h, m, s) = (abs / 3600, abs / 60 % 60, abs % 60);
    if s != 0 {
        format!("{sign}{h:02}:{m:02}:{s:02}")
    } else if m != 0 {
        format!("{sign}{h:02}:{m:02}")
    } else {
        format!("{sign}{h:02}")
    }
}

fn timestamp(micros: i64, with_zone: bool) -> String {
    match micros {
        i64::MAX => return "infinity".into(),
        i64::MIN => return "-infinity".into(),
        _ => {}
    }
    let base = NaiveDateTime::new(epoch(), NaiveTime::MIN);
    let Some(value) = base.checked_add_signed(Duration::microseconds(micros)) else {
        return format!("{micros} µs from 2000-01-01");
    };
    use chrono::Timelike;
    let t = value.time();
    let mut out = format!(
        "{} {:02}:{:02}:{:02}",
        format_date(value.date()).trim_end_matches(" BC"),
        t.hour(),
        t.minute(),
        t.second()
    );
    push_fraction(&mut out, (t.nanosecond() / 1000) as i64);
    if with_zone {
        // Values are stored in UTC and shown in UTC.
        out.push_str("+00");
    }
    if format_date(value.date()).ends_with(" BC") {
        out.push_str(" BC");
    }
    out
}

fn interval(months: i32, days: i32, micros: i64) -> String {
    let mut parts = Vec::new();
    // PostgreSQL writes an explicit '+' on a positive field that follows a
    // negative one, so the sign of each field is unambiguous.
    let mut after_negative = false;
    let mut unit = |n: i32, name: &str, parts: &mut Vec<String>| {
        if n == 0 {
            return;
        }
        let plus = if after_negative && n > 0 { "+" } else { "" };
        parts.push(format!("{plus}{n} {name}{}", if n == 1 { "" } else { "s" }));
        after_negative = n < 0;
    };
    unit(months / 12, "year", &mut parts);
    unit(months % 12, "mon", &mut parts);
    unit(days, "day", &mut parts);
    if micros != 0 || parts.is_empty() {
        let sign = if micros < 0 {
            "-"
        } else if after_negative {
            "+"
        } else {
            ""
        };
        let abs = micros.unsigned_abs();
        let secs = abs / 1_000_000;
        let mut clock = format!(
            "{sign}{:02}:{:02}:{:02}",
            secs / 3600,
            secs / 60 % 60,
            secs % 60
        );
        push_fraction(&mut clock, (abs % 1_000_000) as i64);
        parts.push(clock);
    }
    parts.join(" ")
}

fn inet(r: &mut Reader) -> Option<String> {
    let family = r.u8()?;
    let bits = r.u8()?;
    let is_cidr = r.u8()? != 0;
    let len = r.u8()? as usize;
    let addr = r.take(len)?;
    let (text, full) = match (family, len) {
        (2, 4) => (Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]).to_string(), 32),
        (3, 16) => {
            let bytes: [u8; 16] = addr.try_into().ok()?;
            (Ipv6Addr::from(bytes).to_string(), 128)
        }
        _ => return None,
    };
    Some(if is_cidr || bits != full {
        format!("{text}/{bits}")
    } else {
        text
    })
}

fn hstore(r: &mut Reader) -> Option<String> {
    let count = r.i32()?;
    let quote = |b: &[u8]| format!("\"{}\"", text(b).replace('\\', "\\\\").replace('"', "\\\""));
    let mut pairs = Vec::new();
    for _ in 0..count {
        let key = r.field()??;
        let value = r.field()?;
        pairs.push(format!(
            "{}=>{}",
            quote(key),
            value.map_or("NULL".to_string(), quote)
        ));
    }
    Some(pairs.join(", "))
}

/// Quotes an array element or composite field the way PostgreSQL does.
fn quote_element(value: &str, array: bool) -> String {
    let needs_quotes = value.is_empty()
        || value.chars().any(|c| {
            c == '"' || c == '\\' || c == ',' || c.is_whitespace() || {
                if array {
                    c == '{' || c == '}'
                } else {
                    c == '(' || c == ')'
                }
            }
        })
        || (array && value.eq_ignore_ascii_case("null"));
    if !needs_quotes {
        return value.to_string();
    }
    let escaped = if array {
        value.replace('\\', "\\\\").replace('"', "\\\"")
    } else {
        value.replace('\\', "\\\\").replace('"', "\"\"")
    };
    format!("\"{escaped}\"")
}

fn array(element: &Type, raw: &[u8]) -> Option<String> {
    let mut r = Reader(raw);
    let ndim = r.i32()? as usize;
    r.i32()?; // has-nulls flag
    r.u32()?; // element oid
    if ndim == 0 {
        return Some("{}".into());
    }
    let mut dims = Vec::with_capacity(ndim);
    for _ in 0..ndim {
        let len = r.i32()? as usize;
        r.i32()?; // lower bound
        dims.push(len);
    }
    // Composite and range elements are always quoted; box uses ';' — not handled.
    fn level(element: &Type, r: &mut Reader, dims: &[usize], out: &mut String) -> Option<()> {
        out.push('{');
        for i in 0..dims[0] {
            if i > 0 {
                out.push(',');
            }
            if dims.len() > 1 {
                level(element, r, &dims[1..], out)?;
            } else {
                match r.field()? {
                    None => out.push_str("NULL"),
                    Some(bytes) => out.push_str(&quote_element(&render(element, bytes), true)),
                }
            }
        }
        out.push('}');
        Some(())
    }
    let mut out = String::new();
    level(element, &mut r, &dims, &mut out)?;
    Some(out)
}

fn range(element: &Type, r: &mut Reader) -> Option<String> {
    const EMPTY: u8 = 0x01;
    const LOWER_INCLUSIVE: u8 = 0x02;
    const UPPER_INCLUSIVE: u8 = 0x04;
    const LOWER_INFINITE: u8 = 0x08;
    const UPPER_INFINITE: u8 = 0x10;

    let flags = r.u8()?;
    if flags & EMPTY != 0 {
        return Some("empty".into());
    }
    let mut bound = |infinite: bool| -> Option<String> {
        if infinite {
            return Some(String::new());
        }
        let bytes = r.field()??;
        Some(quote_element(&render(element, bytes), false))
    };
    let lower = bound(flags & LOWER_INFINITE != 0)?;
    let upper = bound(flags & UPPER_INFINITE != 0)?;
    Some(format!(
        "{}{lower},{upper}{}",
        if flags & LOWER_INCLUSIVE != 0 { '[' } else { '(' },
        if flags & UPPER_INCLUSIVE != 0 { ']' } else { ')' },
    ))
}

fn multirange(range_type: &Type, raw: &[u8]) -> Option<String> {
    let Kind::Range(element) = range_type.kind() else {
        return None;
    };
    let mut r = Reader(raw);
    let count = r.i32()?;
    let mut parts = Vec::new();
    for _ in 0..count {
        let bytes = r.field()??;
        parts.push(range(element, &mut Reader(bytes))?);
    }
    Some(format!("{{{}}}", parts.join(",")))
}

fn composite(types: &[&Type], raw: &[u8]) -> Option<String> {
    let mut r = Reader(raw);
    let count = r.i32()? as usize;
    let mut parts = Vec::with_capacity(count);
    for i in 0..count {
        r.u32()?; // field oid
        parts.push(match r.field()? {
            None => String::new(),
            Some(bytes) => match types.get(i) {
                Some(ty) => quote_element(&render(ty, bytes), false),
                None => quote_element(&fallback(bytes), false),
            },
        });
    }
    Some(format!("({})", parts.join(",")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numeric_bytes(digits: &[u16], weight: i16, sign: u16, dscale: u16) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend((digits.len() as i16).to_be_bytes());
        out.extend(weight.to_be_bytes());
        out.extend(sign.to_be_bytes());
        out.extend(dscale.to_be_bytes());
        for d in digits {
            out.extend(d.to_be_bytes());
        }
        out
    }

    #[test]
    fn numeric_keeps_scale_and_sign() {
        let n = |d: &[u16], w, s, ds| numeric(&mut Reader(&numeric_bytes(d, w, s, ds))).unwrap();
        assert_eq!(n(&[1234, 5678], 1, 0, 0), "12345678");
        assert_eq!(n(&[1, 5000], 0, 0, 2), "1.50");
        assert_eq!(n(&[5], -1, 0x4000, 4), "-0.0005");
        assert_eq!(n(&[], 0, 0, 0), "0");
        assert_eq!(n(&[], 0, 0, 3), "0.000");
        assert_eq!(n(&[1], 2, 0, 0), "100000000");
        assert_eq!(n(&[], 0, 0xC000, 0), "NaN");
    }

    #[test]
    fn floats_match_postgres_text() {
        assert_eq!(float(1.5, false), "1.5");
        assert_eq!(float(100.0, false), "100");
        assert_eq!(float(1e20, false), "1e+20");
        assert_eq!(float(1.5e-7, false), "1.5e-07");
        assert_eq!(float(0.1, true), "0.1");
        assert_eq!(float(f64::NAN, false), "NaN");
        assert_eq!(float(f64::NEG_INFINITY, false), "-Infinity");
    }

    #[test]
    fn temporal_values_format_like_psql() {
        assert_eq!(date(0), "2000-01-01");
        assert_eq!(date(i32::MAX), "infinity");
        assert_eq!(time(3_723_500_000), "01:02:03.5");
        assert_eq!(timestamp(0, true), "2000-01-01 00:00:00+00");
        assert_eq!(timestamp(-1, false), "1999-12-31 23:59:59.999999");
        assert_eq!(interval(14, 3, 3_600_000_000), "1 year 2 mons 3 days 01:00:00");
        assert_eq!(interval(0, 0, 0), "00:00:00");
        assert_eq!(interval(0, 1, -1_500_000), "1 day -00:00:01.5");
        assert_eq!(zone(19_800), "+05:30");
    }

    #[test]
    fn unknown_types_fall_back_to_text_or_hex() {
        assert_eq!(fallback(b"plain text"), "plain text");
        assert_eq!(fallback(&[0, 159, 146]), "\\x009f92");
    }

    #[test]
    fn elements_are_quoted_only_when_needed() {
        assert_eq!(quote_element("abc", true), "abc");
        assert_eq!(quote_element("a b", true), "\"a b\"");
        assert_eq!(quote_element("", true), "\"\"");
        assert_eq!(quote_element("null", true), "\"null\"");
        assert_eq!(quote_element("say \"hi\"", true), "\"say \\\"hi\\\"\"");
        assert_eq!(quote_element("say \"hi\"", false), "\"say \"\"hi\"\"\"");
    }
}
