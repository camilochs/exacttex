//! JSON-RPC over stdio: framing in, framing out.
//!
//! The whole transport is here and it is deliberately dull. It reads a
//! `Content-Length` header, reads that many bytes, and hands them on; it writes
//! the same shape back. Nothing in this file knows what a document is.

use std::io::{BufRead, Write};

/// One message read from the stream.
pub struct Message {
    /// `id`, absent for a notification. A notification is never answered.
    pub id: Option<i64>,
    /// The method the client named.
    pub method: String,
    /// `params`, unexamined.
    pub params: xtex_core::json::Value,
}

/// Reads one message, or `None` at end of stream.
///
/// # Errors
///
/// Fails when the stream cannot be read. A malformed frame is reported as
/// invalid data rather than skipped, because a client that framed one message
/// wrongly will frame the next one wrongly too, and continuing would turn a
/// protocol bug into a silent hang.
pub fn read(input: &mut impl BufRead) -> std::io::Result<Option<Message>> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse().ok();
        }
    }
    let Some(length) = length else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "a frame arrived with no Content-Length",
        ));
    };
    let mut body = vec![0u8; length];
    input.read_exact(&mut body)?;

    let Some(value) = xtex_core::json::parse(&body) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "a frame body was not JSON",
        ));
    };
    let method = value
        .get("method")
        .and_then(xtex_core::json::Value::text)
        .unwrap_or_default()
        .to_owned();
    Ok(Some(Message {
        id: value.get("id").and_then(xtex_core::json::Value::integer),
        method,
        params: value
            .get("params")
            .cloned()
            .unwrap_or(xtex_core::json::Value::Null),
    }))
}

/// Writes a framed response body.
///
/// # Errors
///
/// Fails when the stream cannot be written.
pub fn write(output: &mut impl Write, body: &str) -> std::io::Result<()> {
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    output.flush()
}

/// A successful reply to `id`, carrying `result` verbatim.
#[must_use]
pub fn reply(id: i64, result: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#)
}

/// A notification carrying `params` verbatim.
#[must_use]
pub fn notify(method: &str, params: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","method":"{method}","params":{params}}}"#)
}
