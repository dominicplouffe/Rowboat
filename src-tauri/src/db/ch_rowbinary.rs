//! Decoder for ClickHouse's `RowBinaryWithNamesAndTypes` format.
//!
//! The stream is: column count, column names, column type names (all as
//! length-prefixed strings), then rows of raw little-endian values with no
//! framing. Decoding therefore needs each column's type parsed from its name.
//! Values are rendered the way `clickhouse-client` prints them.
//!
//! Decoding is incremental: [`Decoder::push`] accepts bytes as they arrive
//! over HTTP and yields complete rows, so a result can be cut off at the row
//! limit without reading the rest.

use std::collections::HashMap;
use std::fmt::Write;
use std::net::{Ipv4Addr, Ipv6Addr};

use chrono::{DateTime, NaiveDate};

use super::{Cell, Column, ColumnKind};

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    UInt(usize),
    Int(usize),
    Float32,
    Float64,
    Bool,
    String,
    FixedString(usize),
    Date,
    Date32,
    DateTime,
    DateTime64(u32),
    Time,
    Time64(u32),
    Decimal {
        bytes: usize,
        scale: usize,
    },
    Uuid,
    Ipv4,
    Ipv6,
    Enum {
        bytes: usize,
        names: HashMap<i16, String>,
    },
    Nullable(Box<Type>),
    Array(Box<Type>),
    Tuple(Vec<Type>),
    Map(Box<Type>, Box<Type>),
    Variant(Vec<Type>),
    Nothing,
}

#[derive(Debug, PartialEq)]
pub enum DecodeError {
    /// Not an error: more bytes are needed to finish the current item.
    Incomplete,
    Invalid(String),
}

type Decoded<T> = Result<T, DecodeError>;

fn invalid<T>(message: impl Into<String>) -> Decoded<T> {
    Err(DecodeError::Invalid(message.into()))
}

// ---------------------------------------------------------- type parsing ----

/// Splits `a, b(c, d), 'e,f'` at top-level commas.
fn split_args(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quote, mut start) = (0usize, false, 0usize);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if quote => i += 1,
            b'\'' => quote = !quote,
            b'(' if !quote => depth += 1,
            b')' if !quote => depth = depth.saturating_sub(1),
            b',' if !quote && depth == 0 => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if !s[start..].trim().is_empty() {
        parts.push(s[start..].trim());
    }
    parts
}

/// `name Type` (a named tuple element) → `Type`.
fn strip_element_name(arg: &str) -> &str {
    let first_special = arg.find(['(', ' ', '\'']);
    match first_special {
        Some(i) if arg.as_bytes()[i] == b' ' => arg[i + 1..].trim_start(),
        _ => arg,
    }
}

fn unquote(s: &str) -> String {
    let inner = s.trim().trim_start_matches('\'').trim_end_matches('\'');
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn parse_type(name: &str) -> Result<Type, String> {
    let name = name.trim();
    let (base, args) = match name.find('(') {
        Some(open) if name.ends_with(')') => (&name[..open], &name[open + 1..name.len() - 1]),
        _ => (name, ""),
    };
    let inner = |index: usize| -> Result<Type, String> {
        let parts = split_args(args);
        let part = parts
            .get(index)
            .ok_or_else(|| format!("{name}: missing type argument"))?;
        parse_type(part)
    };
    let number = |index: usize| -> Result<usize, String> {
        split_args(args)
            .get(index)
            .and_then(|a| a.parse().ok())
            .ok_or_else(|| format!("{name}: expected a number"))
    };
    let point = || Type::Tuple(vec![Type::Float64, Type::Float64]);
    let array = |t: Type| Type::Array(Box::new(t));

    Ok(match base {
        "UInt8" => Type::UInt(1),
        "UInt16" => Type::UInt(2),
        "UInt32" => Type::UInt(4),
        "UInt64" => Type::UInt(8),
        "UInt128" => Type::UInt(16),
        "UInt256" => Type::UInt(32),
        "Int8" => Type::Int(1),
        "Int16" => Type::Int(2),
        "Int32" => Type::Int(4),
        "Int64" => Type::Int(8),
        "Int128" => Type::Int(16),
        "Int256" => Type::Int(32),
        "Float32" => Type::Float32,
        "Float64" => Type::Float64,
        "Bool" => Type::Bool,
        // JSON arrives as a string because the driver asks for it
        // (output_format_binary_write_json_as_string).
        "String" | "JSON" => Type::String,
        "FixedString" => Type::FixedString(number(0)?),
        "Date" => Type::Date,
        "Date32" => Type::Date32,
        "DateTime" | "DateTime32" => Type::DateTime,
        "DateTime64" => Type::DateTime64(number(0)? as u32),
        "Time" => Type::Time,
        "Time64" => Type::Time64(number(0)? as u32),
        "Decimal" => {
            let precision = number(0)?;
            Type::Decimal {
                bytes: match precision {
                    0..=9 => 4,
                    10..=18 => 8,
                    19..=38 => 16,
                    _ => 32,
                },
                scale: number(1)?,
            }
        }
        "Decimal32" => Type::Decimal { bytes: 4, scale: number(0)? },
        "Decimal64" => Type::Decimal { bytes: 8, scale: number(0)? },
        "Decimal128" => Type::Decimal { bytes: 16, scale: number(0)? },
        "Decimal256" => Type::Decimal { bytes: 32, scale: number(0)? },
        "UUID" => Type::Uuid,
        "IPv4" => Type::Ipv4,
        "IPv6" => Type::Ipv6,
        "Enum8" | "Enum16" | "Enum" => {
            let mut names = HashMap::new();
            let mut max = 0i16;
            for (i, arg) in split_args(args).into_iter().enumerate() {
                // 'label' = value; the value is optional in definitions.
                let (label, value) = match arg.rfind('=') {
                    Some(eq) if arg[..eq].trim_end().ends_with('\'') => (
                        &arg[..eq],
                        arg[eq + 1..]
                            .trim()
                            .parse::<i16>()
                            .map_err(|_| format!("{name}: bad enum value"))?,
                    ),
                    _ => (arg, i as i16 + 1),
                };
                max = max.max(value.abs());
                names.insert(value, unquote(label));
            }
            Type::Enum {
                bytes: if base == "Enum16" || (base == "Enum" && max > 127) { 2 } else { 1 },
                names,
            }
        }
        "Nullable" => Type::Nullable(Box::new(inner(0)?)),
        "LowCardinality" => inner(0)?,
        "SimpleAggregateFunction" => inner(1)?,
        "Array" => array(inner(0)?),
        "Map" => Type::Map(Box::new(inner(0)?), Box::new(inner(1)?)),
        "Tuple" => Type::Tuple(
            split_args(args)
                .into_iter()
                .map(|a| parse_type(strip_element_name(a)))
                .collect::<Result<_, _>>()?,
        ),
        "Nested" => array(Type::Tuple(
            split_args(args)
                .into_iter()
                .map(|a| parse_type(strip_element_name(a)))
                .collect::<Result<_, _>>()?,
        )),
        "Variant" => Type::Variant(
            split_args(args)
                .into_iter()
                .map(parse_type)
                .collect::<Result<_, _>>()?,
        ),
        "Nothing" => Type::Nothing,
        "Point" => point(),
        "Ring" | "LineString" => array(point()),
        "Polygon" | "MultiLineString" => array(array(point())),
        "MultiPolygon" => array(array(array(point()))),
        other if other.starts_with("Interval") => Type::Int(8),
        _ => {
            return Err(format!(
                "Rowboat cannot display the ClickHouse type {name} yet. Cast the column to String, e.g. toString(col)."
            ))
        }
    })
}

pub fn column_kind(ty: &Type) -> ColumnKind {
    match ty {
        Type::UInt(_) | Type::Int(_) | Type::Float32 | Type::Float64 | Type::Decimal { .. } => {
            ColumnKind::Number
        }
        Type::Bool => ColumnKind::Bool,
        Type::Date
        | Type::Date32
        | Type::DateTime
        | Type::DateTime64(_)
        | Type::Time
        | Type::Time64(_) => ColumnKind::Temporal,
        Type::String
        | Type::FixedString(_)
        | Type::Uuid
        | Type::Enum { .. }
        | Type::Ipv4
        | Type::Ipv6 => ColumnKind::Text,
        Type::Nullable(inner) => column_kind(inner),
        _ => ColumnKind::Other,
    }
}

// --------------------------------------------------------------- reading ----

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Decoded<&'a [u8]> {
        if self.data.len() - self.pos < n {
            return Err(DecodeError::Incomplete);
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }
    fn u8(&mut self) -> Decoded<u8> {
        Ok(self.take(1)?[0])
    }
    /// Unsigned LEB128, used for string lengths and collection sizes.
    fn varint(&mut self) -> Decoded<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            value |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        invalid("length prefix is too long")
    }
    fn size(&mut self) -> Decoded<usize> {
        let n = self.varint()?;
        // A single value larger than this means we are out of step with the stream.
        if n > 1 << 31 {
            return invalid("implausible length in the result stream");
        }
        Ok(n as usize)
    }
    fn string(&mut self) -> Decoded<String> {
        let len = self.size()?;
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }
}

/// Decimal text of a little-endian two's-complement integer of any width.
fn integer_text(bytes: &[u8], signed: bool) -> String {
    if bytes.len() <= 16 {
        let mut buf = [0u8; 16];
        buf[..bytes.len()].copy_from_slice(bytes);
        let negative = signed && bytes.last().is_some_and(|b| b & 0x80 != 0);
        if negative {
            buf[bytes.len()..].fill(0xff);
            return i128::from_le_bytes(buf).to_string();
        }
        return u128::from_le_bytes(buf).to_string();
    }
    // 256-bit: long division by 10 over base-2^32 limbs.
    let negative = signed && bytes.last().is_some_and(|b| b & 0x80 != 0);
    let mut limbs: Vec<u32> = bytes
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if negative {
        // Two's complement negate.
        let mut carry = true;
        for limb in &mut limbs {
            let (value, overflow) = (!*limb).overflowing_add(carry as u32);
            *limb = value;
            carry = overflow;
        }
    }
    let mut digits = Vec::new();
    while limbs.iter().any(|&l| l != 0) {
        let mut remainder = 0u64;
        for limb in limbs.iter_mut().rev() {
            let current = (remainder << 32) | *limb as u64;
            *limb = (current / 10) as u32;
            remainder = current % 10;
        }
        digits.push(char::from(b'0' + remainder as u8));
    }
    if digits.is_empty() {
        return "0".into();
    }
    if negative {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

/// Inserts a decimal point `scale` digits from the right.
fn scaled(integer: String, scale: usize) -> String {
    if scale == 0 {
        return integer;
    }
    let (sign, digits) = match integer.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", integer.as_str()),
    };
    let padded = format!("{digits:0>width$}", width = scale + 1);
    let (whole, fraction) = padded.split_at(padded.len() - scale);
    // ClickHouse trims trailing zeros of the fraction.
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    }
}

fn float(value: f64, text: String) -> String {
    if value.is_nan() {
        "nan".into()
    } else if value.is_infinite() {
        if value > 0.0 { "inf" } else { "-inf" }.into()
    } else {
        text
    }
}

fn date_from_days(days: i64) -> String {
    NaiveDate::from_ymd_opt(1970, 1, 1)
        .and_then(|epoch| epoch.checked_add_signed(chrono::Duration::days(days)))
        .map_or_else(
            || format!("{days} days from 1970-01-01"),
            |d| d.format("%Y-%m-%d").to_string(),
        )
}

/// Timestamps are shown in UTC.
fn datetime(seconds: i64, fraction: Option<(u64, u32)>) -> String {
    let Some(value) = DateTime::from_timestamp(seconds, 0) else {
        return format!("{seconds} s from 1970-01-01");
    };
    let mut out = value.format("%Y-%m-%d %H:%M:%S").to_string();
    if let Some((ticks, precision)) = fraction {
        if precision > 0 {
            let _ = write!(out, ".{ticks:0width$}", width = precision as usize);
        }
    }
    out
}

fn clock(seconds: i64) -> String {
    let sign = if seconds < 0 { "-" } else { "" };
    let s = seconds.unsigned_abs();
    format!("{sign}{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Decodes one value. `nested` is true inside arrays, tuples and maps, where
/// ClickHouse quotes strings and writes NULL literally.
fn value(ty: &Type, r: &mut Reader, nested: bool) -> Decoded<Option<String>> {
    let quoted = |text: String| if nested { quote(&text) } else { text };
    Ok(Some(match ty {
        Type::UInt(n) => integer_text(r.take(*n)?, false),
        Type::Int(n) => integer_text(r.take(*n)?, true),
        Type::Float32 => {
            let v = f32::from_le_bytes(r.take(4)?.try_into().unwrap());
            float(v as f64, v.to_string())
        }
        Type::Float64 => {
            let v = f64::from_le_bytes(r.take(8)?.try_into().unwrap());
            float(v, v.to_string())
        }
        Type::Bool => if r.u8()? != 0 { "true" } else { "false" }.to_string(),
        Type::String => quoted(r.string()?),
        Type::FixedString(n) => {
            let bytes = r.take(*n)?;
            let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
            quoted(String::from_utf8_lossy(&bytes[..end]).into_owned())
        }
        Type::Date => {
            let days = u16::from_le_bytes(r.take(2)?.try_into().unwrap());
            quoted(date_from_days(days as i64))
        }
        Type::Date32 => {
            let days = i32::from_le_bytes(r.take(4)?.try_into().unwrap());
            quoted(date_from_days(days as i64))
        }
        Type::DateTime => {
            let seconds = u32::from_le_bytes(r.take(4)?.try_into().unwrap());
            quoted(datetime(seconds as i64, None))
        }
        Type::DateTime64(precision) => {
            let ticks = i64::from_le_bytes(r.take(8)?.try_into().unwrap());
            let unit = 10i64.pow((*precision).min(18));
            quoted(datetime(
                ticks.div_euclid(unit),
                Some((ticks.rem_euclid(unit) as u64, *precision)),
            ))
        }
        Type::Time => {
            let seconds = i32::from_le_bytes(r.take(4)?.try_into().unwrap());
            quoted(clock(seconds as i64))
        }
        Type::Time64(precision) => {
            let ticks = i64::from_le_bytes(r.take(8)?.try_into().unwrap());
            let unit = 10i64.pow((*precision).min(18));
            let mut out = clock(ticks / unit);
            if *precision > 0 {
                let _ = write!(
                    out,
                    ".{:0width$}",
                    (ticks % unit).abs(),
                    width = *precision as usize
                );
            }
            quoted(out)
        }
        Type::Decimal { bytes, scale } => scaled(integer_text(r.take(*bytes)?, true), *scale),
        Type::Uuid => {
            // Two little-endian UInt64 halves, high half first.
            let b = r.take(16)?;
            let hex: String = b[..8]
                .iter()
                .rev()
                .chain(b[8..].iter().rev())
                .map(|x| format!("{x:02x}"))
                .collect();
            quoted(format!(
                "{}-{}-{}-{}-{}",
                &hex[0..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..32]
            ))
        }
        Type::Ipv4 => {
            let v = u32::from_le_bytes(r.take(4)?.try_into().unwrap());
            quoted(Ipv4Addr::from(v).to_string())
        }
        Type::Ipv6 => {
            let bytes: [u8; 16] = r.take(16)?.try_into().unwrap();
            quoted(Ipv6Addr::from(bytes).to_string())
        }
        Type::Enum { bytes, names } => {
            let raw = if *bytes == 1 {
                r.u8()? as i8 as i16
            } else {
                i16::from_le_bytes(r.take(2)?.try_into().unwrap())
            };
            quoted(names.get(&raw).cloned().unwrap_or_else(|| raw.to_string()))
        }
        Type::Nullable(inner) => {
            if r.u8()? != 0 {
                return Ok(if nested { Some("NULL".into()) } else { None });
            }
            return value(inner, r, nested);
        }
        Type::Array(element) => {
            let len = r.size()?;
            let mut parts = Vec::with_capacity(len.min(1024));
            for _ in 0..len {
                parts.push(value(element, r, true)?.unwrap_or_else(|| "NULL".into()));
            }
            format!("[{}]", parts.join(","))
        }
        Type::Tuple(elements) => {
            let mut parts = Vec::with_capacity(elements.len());
            for element in elements {
                parts.push(value(element, r, true)?.unwrap_or_else(|| "NULL".into()));
            }
            format!("({})", parts.join(","))
        }
        Type::Map(key, val) => {
            let len = r.size()?;
            let mut parts = Vec::with_capacity(len.min(1024));
            for _ in 0..len {
                let k = value(key, r, true)?.unwrap_or_else(|| "NULL".into());
                let v = value(val, r, true)?.unwrap_or_else(|| "NULL".into());
                parts.push(format!("{k}:{v}"));
            }
            format!("{{{}}}", parts.join(","))
        }
        Type::Variant(variants) => {
            let index = r.u8()?;
            if index == 255 {
                return Ok(if nested { Some("NULL".into()) } else { None });
            }
            let Some(variant) = variants.get(index as usize) else {
                return invalid("unknown Variant discriminator");
            };
            return value(variant, r, nested);
        }
        Type::Nothing => return Ok(if nested { Some("NULL".into()) } else { None }),
    }))
}

// --------------------------------------------------------------- decoder ----

/// Incremental decoder. Feed it bytes; take rows out.
#[derive(Default)]
pub struct Decoder {
    buffer: Vec<u8>,
    types: Option<Vec<Type>>,
    pub columns: Vec<Column>,
    /// Set once the server's exception text has started; see [`exception_at`].
    in_exception: bool,
    finished: bool,
}

enum Sniff {
    Exception,
    NotException,
    /// What has arrived so far could still become an exception.
    NeedMore,
}

/// ClickHouse can fail after it has started streaming rows. It then appends
/// the exception as plain text right after the last complete row, optionally
/// wrapped in an `__exception__` marker. Rows are not framed, so without this
/// check that text would be decoded as (garbage) rows.
///
/// Looks at the bytes at a row boundary: `Code: <digits>. DB::Exception: `.
fn exception_at(data: &[u8]) -> Sniff {
    const MARKER: &[u8] = b"__exception__";
    let data = match data {
        [b'\r', b'\n', rest @ ..] => rest,
        [b'\r'] => return Sniff::NeedMore,
        other => other,
    };
    if data.starts_with(MARKER) {
        return Sniff::Exception;
    }
    if MARKER.starts_with(data) {
        return Sniff::NeedMore;
    }
    let pattern: [&[u8]; 2] = [b"Code: ", b". DB::Exception: "];
    let mut pos = 0;
    for (i, expected) in pattern.iter().enumerate() {
        for &byte in expected.iter() {
            match data.get(pos) {
                None => return Sniff::NeedMore,
                Some(&b) if b == byte => pos += 1,
                Some(_) => return Sniff::NotException,
            }
        }
        if i == 0 {
            let digits = data[pos..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
            if digits == 0 || digits > 6 {
                return if pos + digits == data.len() && digits <= 6 {
                    Sniff::NeedMore
                } else {
                    Sniff::NotException
                };
            }
            pos += digits;
        }
    }
    Sniff::Exception
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// True once the header (names and types) has been read.
    pub fn has_header(&self) -> bool {
        self.types.is_some()
    }

    /// Bytes received but not yet decoded into a row.
    pub fn pending(&self) -> &[u8] {
        &self.buffer
    }

    fn header(data: &[u8]) -> Decoded<(usize, Vec<Column>, Vec<Type>)> {
        let mut r = Reader { data, pos: 0 };
        let count = r.size()?;
        if count > 100_000 {
            return invalid("implausible column count in the result stream");
        }
        let names: Vec<String> = (0..count).map(|_| r.string()).collect::<Decoded<_>>()?;
        let type_names: Vec<String> = (0..count).map(|_| r.string()).collect::<Decoded<_>>()?;
        let types: Vec<Type> = type_names
            .iter()
            .map(|t| parse_type(t).map_err(DecodeError::Invalid))
            .collect::<Decoded<_>>()?;
        let columns = names
            .into_iter()
            .zip(type_names)
            .zip(&types)
            .map(|((name, type_name), ty)| Column {
                name,
                type_name,
                kind: column_kind(ty),
            })
            .collect();
        Ok((r.pos, columns, types))
    }

    /// The exception text the server appended to the stream, if any.
    pub fn exception(&self) -> Option<String> {
        self.in_exception
            .then(|| String::from_utf8_lossy(&self.buffer).into_owned())
    }

    /// Call when the stream has ended, to decode what was held back while
    /// waiting to see whether it was the start of an exception.
    pub fn finish(&mut self, max_rows: usize) -> Result<Vec<Vec<Cell>>, String> {
        self.finished = true;
        self.push(&[], max_rows)
    }

    /// Adds bytes and returns up to `max_rows` complete rows (0 = no cap).
    /// Rows beyond the cap stay buffered for the next call.
    pub fn push(&mut self, bytes: &[u8], max_rows: usize) -> Result<Vec<Vec<Cell>>, String> {
        self.buffer.extend_from_slice(bytes);
        if self.in_exception {
            return Ok(vec![]);
        }
        let mut consumed = 0;
        if self.types.is_none() {
            match Self::header(&self.buffer) {
                Ok((used, columns, types)) => {
                    consumed = used;
                    self.columns = columns;
                    self.types = Some(types);
                }
                Err(DecodeError::Incomplete) => return Ok(vec![]),
                Err(DecodeError::Invalid(message)) => return Err(message),
            }
        }
        let types = self.types.as_ref().expect("header was read");
        let mut rows = Vec::new();
        // A result with no columns has no rows to delimit.
        while consumed < self.buffer.len() && (max_rows == 0 || rows.len() < max_rows) {
            match exception_at(&self.buffer[consumed..]) {
                Sniff::Exception => {
                    self.in_exception = true;
                    break;
                }
                Sniff::NeedMore if !self.finished => break,
                _ => {}
            }
            if types.is_empty() {
                break;
            }
            let mut r = Reader {
                data: &self.buffer[consumed..],
                pos: 0,
            };
            let row: Decoded<Vec<Cell>> = types.iter().map(|ty| value(ty, &mut r, false)).collect();
            match row {
                Ok(row) => {
                    consumed += r.pos;
                    rows.push(row);
                }
                Err(DecodeError::Incomplete) => break,
                Err(DecodeError::Invalid(message)) => {
                    self.buffer.drain(..consumed);
                    return Err(message);
                }
            }
        }
        self.buffer.drain(..consumed);
        Ok(rows)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Builds RowBinary bytes the way the server does, for tests.

    pub fn varint(mut n: u64, out: &mut Vec<u8>) {
        loop {
            let byte = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    pub fn string(s: &str, out: &mut Vec<u8>) {
        varint(s.len() as u64, out);
        out.extend_from_slice(s.as_bytes());
    }

    pub fn header(columns: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        varint(columns.len() as u64, &mut out);
        for (name, _) in columns {
            string(name, &mut out);
        }
        for (_, ty) in columns {
            string(ty, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    /// Decodes a single value of the given type from `bytes`.
    fn one(type_name: &str, bytes: &[u8]) -> Option<String> {
        let mut data = header(&[("c", type_name)]);
        data.extend_from_slice(bytes);
        let mut decoder = Decoder::new();
        let rows = decoder
            .push(&data, 0)
            .unwrap_or_else(|e| panic!("{type_name}: {e}"));
        assert_eq!(rows.len(), 1, "{type_name}");
        assert!(decoder.pending().is_empty(), "{type_name}: bytes left over");
        rows[0][0].clone()
    }

    fn text(type_name: &str, bytes: &[u8]) -> String {
        one(type_name, bytes).expect("non-null")
    }

    fn s(value: &str) -> Vec<u8> {
        let mut out = Vec::new();
        string(value, &mut out);
        out
    }

    #[test]
    fn integers_of_every_width() {
        assert_eq!(text("UInt8", &[255]), "255");
        assert_eq!(text("Int8", &[0xff]), "-1");
        assert_eq!(text("UInt16", &1000u16.to_le_bytes()), "1000");
        assert_eq!(text("Int32", &(-123456i32).to_le_bytes()), "-123456");
        assert_eq!(
            text("UInt64", &u64::MAX.to_le_bytes()),
            "18446744073709551615"
        );
        assert_eq!(
            text("Int64", &i64::MIN.to_le_bytes()),
            "-9223372036854775808"
        );
        assert_eq!(
            text("UInt128", &u128::MAX.to_le_bytes()),
            u128::MAX.to_string()
        );
        assert_eq!(text("Int128", &(-5i128).to_le_bytes()), "-5");

        let mut max256 = [0xffu8; 32];
        assert_eq!(
            text("UInt256", &max256),
            "115792089237316195423570985008687907853269984665640564039457584007913129639935"
        );
        assert_eq!(text("Int256", &max256), "-1");
        max256[31] = 0x7f;
        assert_eq!(
            text("Int256", &max256),
            "57896044618658097711785492504343953926634992332820282019728792003956564819967"
        );
        assert_eq!(text("UInt256", &[0u8; 32]), "0");
    }

    #[test]
    fn floats_and_bools() {
        assert_eq!(text("Float64", &1.5f64.to_le_bytes()), "1.5");
        assert_eq!(text("Float32", &0.1f32.to_le_bytes()), "0.1");
        assert_eq!(text("Float64", &f64::NAN.to_le_bytes()), "nan");
        assert_eq!(text("Float64", &f64::NEG_INFINITY.to_le_bytes()), "-inf");
        assert_eq!(text("Bool", &[1]), "true");
        assert_eq!(text("Bool", &[0]), "false");
    }

    #[test]
    fn strings() {
        assert_eq!(text("String", &s("héllo")), "héllo");
        assert_eq!(text("String", &s("")), "");
        assert_eq!(text("FixedString(5)", b"ab\0\0\0"), "ab");
        assert_eq!(text("LowCardinality(String)", &s("lc")), "lc");
        // A string longer than 127 bytes needs a two-byte length prefix.
        let long = "x".repeat(300);
        assert_eq!(text("String", &s(&long)), long);
    }

    #[test]
    fn dates_and_times_in_utc() {
        assert_eq!(text("Date", &19782u16.to_le_bytes()), "2024-02-29");
        assert_eq!(text("Date32", &(-1i32).to_le_bytes()), "1969-12-31");
        assert_eq!(
            text("DateTime", &1709212455u32.to_le_bytes()),
            "2024-02-29 13:14:15"
        );
        assert_eq!(
            text("DateTime('Europe/Paris')", &1709212455u32.to_le_bytes()),
            "2024-02-29 13:14:15"
        );
        assert_eq!(
            text("DateTime64(3)", &1709212455123i64.to_le_bytes()),
            "2024-02-29 13:14:15.123"
        );
        assert_eq!(
            text("DateTime64(6, 'UTC')", &(-1i64).to_le_bytes()),
            "1969-12-31 23:59:59.999999"
        );
        assert_eq!(
            text("DateTime64(0)", &60i64.to_le_bytes()),
            "1970-01-01 00:01:00"
        );
    }

    #[test]
    fn decimals_place_the_point_by_scale() {
        assert_eq!(text("Decimal(9, 2)", &12345i32.to_le_bytes()), "123.45");
        assert_eq!(text("Decimal(18, 4)", &(-5i64).to_le_bytes()), "-0.0005");
        assert_eq!(
            text("Decimal(38, 10)", &10_000_000_000i128.to_le_bytes()),
            "1"
        );
        assert_eq!(text("Decimal64(3)", &1500i64.to_le_bytes()), "1.5");
        let mut wide = [0u8; 32];
        wide[..4].copy_from_slice(&12345u32.to_le_bytes());
        assert_eq!(text("Decimal(76, 2)", &wide), "123.45");
    }

    #[test]
    fn uuid_and_ip_addresses() {
        // 61f0c404-5cb3-11e7-907b-a6006ad3dba0 as two little-endian halves.
        let mut uuid = Vec::new();
        uuid.extend(0x61f0c4045cb311e7u64.to_le_bytes());
        uuid.extend(0x907ba6006ad3dba0u64.to_le_bytes());
        assert_eq!(text("UUID", &uuid), "61f0c404-5cb3-11e7-907b-a6006ad3dba0");

        assert_eq!(
            text(
                "IPv4",
                &u32::from(Ipv4Addr::new(192, 168, 1, 10)).to_le_bytes()
            ),
            "192.168.1.10"
        );
        let v6: Ipv6Addr = "2001:db8::1".parse().unwrap();
        assert_eq!(text("IPv6", &v6.octets()), "2001:db8::1");
    }

    #[test]
    fn enums_show_their_labels() {
        assert_eq!(text("Enum8('active' = 1, 'it''s' = 2)", &[1]), "active");
        assert_eq!(text("Enum8('a' = -1, 'b, c' = 2)", &[2]), "b, c");
        assert_eq!(text("Enum8('a' = -1, 'b' = 2)", &[0xff]), "a");
        assert_eq!(text("Enum16('big' = 1000)", &1000i16.to_le_bytes()), "big");
        assert_eq!(
            text("Enum8('a' = 1)", &[9]),
            "9",
            "an unknown value shows its number"
        );
    }

    #[test]
    fn nullable_values() {
        assert_eq!(one("Nullable(Int32)", &[1]), None);
        let mut present = vec![0];
        present.extend(7i32.to_le_bytes());
        assert_eq!(one("Nullable(Int32)", &present).as_deref(), Some("7"));
        assert_eq!(one("Nullable(Nothing)", &[1]), None);
        assert_eq!(one("LowCardinality(Nullable(String))", &[1]), None);
    }

    #[test]
    fn arrays_tuples_and_maps_print_like_the_client() {
        let mut array = vec![3];
        for n in [1u8, 2, 3] {
            array.push(n);
        }
        assert_eq!(text("Array(UInt8)", &array), "[1,2,3]");
        assert_eq!(text("Array(Nothing)", &[0]), "[]");

        let mut strings = vec![2];
        strings.extend(s("a"));
        strings.extend(s("it's"));
        assert_eq!(text("Array(String)", &strings), "['a','it\\'s']");

        let mut nullable = vec![2, 0, 5, 1];
        nullable.truncate(4);
        assert_eq!(text("Array(Nullable(UInt8))", &nullable), "[5,NULL]");

        let mut nested = vec![2, 1, 9, 0];
        nested.truncate(4);
        assert_eq!(text("Array(Array(UInt8))", &nested), "[[9],[]]");

        let mut tuple = Vec::new();
        tuple.extend(1i32.to_le_bytes());
        tuple.extend(s("x"));
        assert_eq!(text("Tuple(Int32, String)", &tuple), "(1,'x')");
        assert_eq!(text("Tuple(id Int32, name String)", &tuple), "(1,'x')");

        let mut map = vec![1];
        map.extend(s("k"));
        map.extend(2u64.to_le_bytes());
        assert_eq!(text("Map(String, UInt64)", &map), "{'k':2}");

        let mut point = Vec::new();
        point.extend(1.5f64.to_le_bytes());
        point.extend(2f64.to_le_bytes());
        assert_eq!(text("Point", &point), "(1.5,2)");
    }

    #[test]
    fn wrappers_and_parameters_are_understood() {
        assert_eq!(
            parse_type("SimpleAggregateFunction(sum, UInt64)").unwrap(),
            Type::UInt(8)
        );
        assert_eq!(parse_type("IntervalDay").unwrap(), Type::Int(8));
        assert_eq!(
            parse_type("Nested(a UInt8, b String)").unwrap(),
            Type::Array(Box::new(Type::Tuple(vec![Type::UInt(1), Type::String])))
        );
        assert_eq!(
            parse_type("Map(LowCardinality(String), Array(Nullable(DateTime64(3, 'UTC'))))")
                .unwrap(),
            Type::Map(
                Box::new(Type::String),
                Box::new(Type::Array(Box::new(Type::Nullable(Box::new(
                    Type::DateTime64(3)
                )))))
            )
        );
    }

    #[test]
    fn unsupported_types_say_what_to_do() {
        let error = parse_type("AggregateFunction(uniq, UInt64)").unwrap_err();
        assert!(error.contains("AggregateFunction(uniq, UInt64)") && error.contains("toString"));
        let mut decoder = Decoder::new();
        assert!(decoder.push(&header(&[("c", "Dynamic")]), 0).is_err());
    }

    #[test]
    fn rows_arrive_across_arbitrary_chunk_boundaries() {
        let mut data = header(&[
            ("id", "UInt32"),
            ("name", "String"),
            ("tags", "Array(String)"),
        ]);
        for i in 0..50u32 {
            data.extend(i.to_le_bytes());
            string(&format!("name {i}"), &mut data);
            data.push(2);
            string("a", &mut data);
            string(&"b".repeat(i as usize), &mut data);
        }
        // Every possible chunk size, including splits inside the header,
        // inside length prefixes and inside values.
        for chunk in [1, 2, 3, 7, 64, data.len()] {
            let mut decoder = Decoder::new();
            let mut rows = Vec::new();
            for piece in data.chunks(chunk) {
                rows.extend(decoder.push(piece, 0).unwrap());
            }
            assert_eq!(rows.len(), 50, "chunk size {chunk}");
            assert_eq!(rows[49][1].as_deref(), Some("name 49"));
            assert_eq!(rows[3][2].as_deref(), Some("['a','bbb']"));
            assert_eq!(decoder.columns.len(), 3);
            assert_eq!(decoder.columns[0].kind, ColumnKind::Number);
            assert!(decoder.pending().is_empty());
        }
    }

    #[test]
    fn the_row_cap_leaves_the_rest_buffered() {
        let mut data = header(&[("n", "UInt8")]);
        data.extend(0..10u8);
        let mut decoder = Decoder::new();
        assert_eq!(decoder.push(&data, 4).unwrap().len(), 4);
        assert_eq!(decoder.pending().len(), 6);
        assert_eq!(decoder.push(&[], 0).unwrap().len(), 6);
    }

    const EXCEPTION: &[u8] =
        b"Code: 241. DB::Exception: Memory limit (for query) exceeded. (MEMORY_LIMIT_EXCEEDED) (version 26.8.1.1)\n";

    #[test]
    fn an_exception_appended_mid_stream_is_not_decoded_as_rows() {
        // UInt64 rows: without the check, the 104 bytes of text would decode
        // as 13 plausible-looking numbers.
        let mut data = header(&[("n", "UInt64")]);
        data.extend(1u64.to_le_bytes());
        data.extend(2u64.to_le_bytes());
        data.extend_from_slice(EXCEPTION);
        for chunk in [1, 5, 8, 13, data.len()] {
            let mut decoder = Decoder::new();
            let mut rows = Vec::new();
            for piece in data.chunks(chunk) {
                rows.extend(decoder.push(piece, 0).unwrap());
            }
            rows.extend(decoder.finish(0).unwrap());
            assert_eq!(rows.len(), 2, "chunk size {chunk}");
            let text = decoder.exception().expect("exception detected");
            assert!(
                text.starts_with("Code: 241. DB::Exception: Memory limit"),
                "{text}"
            );
        }
    }

    #[test]
    fn the_tagged_exception_block_is_detected_too() {
        let mut data = header(&[("n", "UInt8")]);
        data.push(7);
        data.extend_from_slice(
            b"\r\n__exception__\r\nabcdefghijklmnop\r\nCode: 395. DB::Exception: boom\r\n",
        );
        let mut decoder = Decoder::new();
        assert_eq!(decoder.push(&data, 0).unwrap().len(), 1);
        assert!(decoder.exception().unwrap().contains("Code: 395"));
    }

    #[test]
    fn data_that_merely_resembles_an_exception_is_still_data() {
        // A String value containing exception text sits behind a length
        // prefix, so it never starts at a row boundary.
        let mut data = header(&[("exception", "String")]);
        string(std::str::from_utf8(EXCEPTION).unwrap(), &mut data);
        let mut decoder = Decoder::new();
        let mut rows = decoder.push(&data, 0).unwrap();
        rows.extend(decoder.finish(0).unwrap());
        assert_eq!(rows.len(), 1);
        assert!(decoder.exception().is_none());

        // Bytes that start like "Code: " but are numbers.
        let mut numbers = header(&[("n", "UInt64")]);
        numbers.extend_from_slice(b"Code: 12");
        numbers.extend_from_slice(b"Code: ab");
        let mut decoder = Decoder::new();
        let mut rows = decoder.push(&numbers, 0).unwrap();
        rows.extend(decoder.finish(0).unwrap());
        assert_eq!(rows.len(), 2);
        assert!(decoder.exception().is_none());
    }

    #[test]
    fn garbage_is_reported_not_misread() {
        let mut data = header(&[("s", "String")]);
        // A length prefix claiming 2^40 bytes.
        data.extend([0x80, 0x80, 0x80, 0x80, 0x80, 0x20]);
        assert!(Decoder::new().push(&data, 0).is_err());
    }

    #[test]
    fn a_header_with_no_columns_yields_no_rows() {
        let mut decoder = Decoder::new();
        assert!(decoder.push(&header(&[]), 0).unwrap().is_empty());
        assert!(decoder.has_header());
    }
}
