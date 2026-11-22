//! The private channel between `hoprs mcp` (the relay) and the open hoprs app: one JSON
//! object per line, UTF-8, at most [`MAX_LINE`] bytes a line.
//!
//! The relay speaks first: [`Hello`]. The app answers with a [`HelloReply`]; on a
//! mismatch it closes. Then the relay sends [`ToApp`] (a call with an id of its own, or
//! the cancel of one) and the app sends [`ToRelay`] (the answer to a call, or a goodbye
//! before it closes). Ids are the relay's and unique per connection, so calls overlap.
//!
//! A line can carry file content (`write_file`'s `content`, `read_file`'s text, a
//! command's output): the reader enforces the line limit while it reads, and nothing in
//! this module logs or prints a message's content (the `Debug` of the message types
//! shows the tool, the id and lengths only).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{Error, ErrorKind, Result};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

/// The channel's version number, in the relay's hello.
pub const PROTOCOL: u32 = 1;
/// The longest line either side accepts or writes, newline excluded (8 MiB).
pub const MAX_LINE: usize = 8 * 1024 * 1024;
/// The longest hello the app reads (4 KiB): a hello holds a number and a version, and
/// [`MAX_LINE`] applies only after a successful one.
pub const HELLO_MAX: usize = 4 * 1024;

/// The relay's first line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub hoprs_agent: u32,
    pub version: String,
}

/// The app's answer to the [`Hello`]. `version` is the app's own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "hello", rename_all = "lowercase")]
pub enum HelloReply {
    Ok { version: String },
    Mismatch { version: String },
}

/// What the relay sends after the hello.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToApp {
    Call {
        call: u64,
        tool: String,
        args: serde_json::Value,
    },
    Cancel {
        cancel: u64,
    },
}

/// The outcome of one call: text blocks, or the text of an error.
#[derive(Clone, PartialEq, Eq)]
pub enum Reply {
    Ok(Vec<String>),
    Err(String),
}

/// What the app sends: the answer to a call, or a goodbye before it closes.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToRelay {
    Answer {
        reply: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Bye {
        bye: ByeReason,
    },
}

/// Why the app is closing the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ByeReason {
    /// The user turned agent access off.
    Off,
    /// hoprs is quitting.
    Quit,
}

impl ToRelay {
    /// The line that answers call `id`.
    pub fn answer(id: u64, reply: Reply) -> ToRelay {
        match reply {
            Reply::Ok(blocks) => ToRelay::Answer {
                reply: id,
                ok: true,
                blocks,
                error: None,
            },
            Reply::Err(error) => ToRelay::Answer {
                reply: id,
                ok: false,
                blocks: Vec::new(),
                error: Some(error),
            },
        }
    }

    /// The id and outcome of an answer; `None` for a goodbye. A failed answer without
    /// text is still a failure.
    pub fn into_reply(self) -> Option<(u64, Reply)> {
        match self {
            ToRelay::Answer {
                reply,
                ok: true,
                blocks,
                ..
            } => Some((reply, Reply::Ok(blocks))),
            ToRelay::Answer {
                reply,
                ok: false,
                error,
                ..
            } => Some((reply, Reply::Err(error.unwrap_or_default()))),
            ToRelay::Bye { .. } => None,
        }
    }
}

/// How the app answers a relay's hello: the same channel number and the same version
/// string is `Ok`. Relay and app are one binary, so a different version means the
/// running window is older (or newer) than the program the agent started.
pub fn hello_reply(theirs: &Hello) -> HelloReply {
    let version = env!("CARGO_PKG_VERSION").to_string();
    if theirs.hoprs_agent == PROTOCOL && theirs.version == version {
        HelloReply::Ok { version }
    } else {
        HelloReply::Mismatch { version }
    }
}

/// Reads one line, without its `\n` (or `\r\n`). `None` at end-of-file; a last line
/// without a newline is returned. A line over [`MAX_LINE`] bytes, or one that is not
/// UTF-8, is `InvalidData`; the reader never holds more than [`MAX_LINE`] bytes of it.
/// Both ends read messages ([`read_message`]); this is for tests that read raw lines.
#[cfg(test)]
pub async fn read_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<String>> {
    read_line_within(reader, MAX_LINE).await
}

async fn read_line_within<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max: usize,
) -> Result<Option<String>> {
    let mut line: Vec<u8> = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }
        match chunk.iter().position(|b| *b == b'\n') {
            Some(at) => {
                if line.len() + at > max + 1 {
                    return Err(too_long());
                }
                line.extend_from_slice(&chunk[..at]);
                reader.consume(at + 1);
                break;
            }
            None => {
                let taken = chunk.len();
                // One byte of slack: the line may end in "\r" that is dropped below.
                if line.len() + taken > max + 1 {
                    return Err(too_long());
                }
                line.extend_from_slice(chunk);
                reader.consume(taken);
            }
        }
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    if line.len() > max {
        return Err(too_long());
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|_| Error::new(ErrorKind::InvalidData, "a line of the channel is not UTF-8"))
}

fn too_long() -> Error {
    Error::new(ErrorKind::InvalidData, "a line of the channel is too long")
}

/// Reads one line and parses it as one JSON object of type `T`. A line that is not an
/// object, or not a `T`, is `InvalidData`. The error text never carries the line.
pub async fn read_message<R: AsyncBufRead + Unpin, T: DeserializeOwned>(
    reader: &mut R,
) -> Result<Option<T>> {
    read_message_within(reader, MAX_LINE).await
}

/// Reads the relay's [`Hello`], like [`read_message`] but at most [`HELLO_MAX`] bytes.
pub async fn read_hello<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<Hello>> {
    read_message_within(reader, HELLO_MAX).await
}

async fn read_message_within<R: AsyncBufRead + Unpin, T: DeserializeOwned>(
    reader: &mut R,
    max: usize,
) -> Result<Option<T>> {
    let Some(line) = read_line_within(reader, max).await? else {
        return Ok(None);
    };
    if !line.trim_start().starts_with('{') {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "a line of the channel is not a JSON object",
        ));
    }
    serde_json::from_str(&line).map(Some).map_err(|e| {
        // `e`'s own text can quote the input: keep only what it classifies.
        let what = if e.is_syntax() || e.is_eof() {
            "is not valid JSON"
        } else {
            "is not a message this version knows"
        };
        Error::new(
            ErrorKind::InvalidData,
            format!("a line of the channel {what}"),
        )
    })
}

/// Writes `message` as one line and flushes. A message over [`MAX_LINE`] bytes is
/// `InvalidInput` and nothing is written.
pub async fn write_line<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    message: &T,
) -> Result<()> {
    let mut bytes =
        serde_json::to_vec(message).map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
    if bytes.len() > MAX_LINE {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "a message of the channel is too long",
        ));
    }
    // serde_json escapes control characters, so the message is one line.
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await
}

impl fmt::Debug for ToApp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToApp::Call { call, tool, args } => f
                .debug_struct("Call")
                .field("call", call)
                .field("tool", tool)
                .field("args", &args.as_object().map_or(0, |o| o.len()))
                .finish(),
            ToApp::Cancel { cancel } => f.debug_struct("Cancel").field("cancel", cancel).finish(),
        }
    }
}

impl fmt::Debug for Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reply::Ok(blocks) => f
                .debug_struct("Ok")
                .field("blocks", &blocks.len())
                .field("bytes", &blocks.iter().map(String::len).sum::<usize>())
                .finish(),
            Reply::Err(text) => f.debug_struct("Err").field("bytes", &text.len()).finish(),
        }
    }
}

impl fmt::Debug for ToRelay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToRelay::Answer {
                reply,
                ok,
                blocks,
                error,
            } => f
                .debug_struct("Answer")
                .field("reply", reply)
                .field("ok", ok)
                .field("blocks", &blocks.len())
                .field(
                    "bytes",
                    &(blocks.iter().map(String::len).sum::<usize>()
                        + error.as_ref().map_or(0, String::len)),
                )
                .finish(),
            ToRelay::Bye { bye } => f.debug_struct("Bye").field("bye", bye).finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::ErrorKind;

    fn text<T: Serialize>(m: &T) -> String {
        serde_json::to_string(m).unwrap()
    }

    #[test]
    fn wire_shapes() {
        // relay -> app: the hello
        let hello = Hello {
            hoprs_agent: 1,
            version: "1.6.0".into(),
        };
        assert_eq!(text(&hello), r#"{"hoprs_agent":1,"version":"1.6.0"}"#);
        assert_eq!(serde_json::from_str::<Hello>(&text(&hello)).unwrap(), hello);

        // app -> relay: the answer to the hello
        let ok = HelloReply::Ok {
            version: "1.6.0".into(),
        };
        assert_eq!(text(&ok), r#"{"hello":"ok","version":"1.6.0"}"#);
        let bad = HelloReply::Mismatch {
            version: "1.6.1".into(),
        };
        assert_eq!(text(&bad), r#"{"hello":"mismatch","version":"1.6.1"}"#);
        for m in [ok, bad] {
            assert_eq!(serde_json::from_str::<HelloReply>(&text(&m)).unwrap(), m);
        }

        // relay -> app: calls and cancels
        let call = ToApp::Call {
            call: 7,
            tool: "stat".into(),
            args: json!({"site":"a","path":"/"}),
        };
        assert_eq!(
            text(&call),
            r#"{"call":7,"tool":"stat","args":{"path":"/","site":"a"}}"#
        );
        let cancel = ToApp::Cancel { cancel: 7 };
        assert_eq!(text(&cancel), r#"{"cancel":7}"#);
        for m in [call, cancel] {
            assert_eq!(serde_json::from_str::<ToApp>(&text(&m)).unwrap(), m);
        }

        // app -> relay: answers and goodbyes
        let ok = ToRelay::answer(7, Reply::Ok(vec!["{\"ok\":true}".into(), "text".into()]));
        assert_eq!(
            text(&ok),
            r#"{"reply":7,"ok":true,"blocks":["{\"ok\":true}","text"]}"#
        );
        let err = ToRelay::answer(8, Reply::Err("nope".into()));
        assert_eq!(text(&err), r#"{"reply":8,"ok":false,"error":"nope"}"#);
        let empty = ToRelay::answer(9, Reply::Ok(Vec::new()));
        assert_eq!(text(&empty), r#"{"reply":9,"ok":true}"#);
        let off = ToRelay::Bye {
            bye: ByeReason::Off,
        };
        assert_eq!(text(&off), r#"{"bye":"off"}"#);
        assert_eq!(
            text(&ToRelay::Bye {
                bye: ByeReason::Quit
            }),
            r#"{"bye":"quit"}"#
        );
        for m in [ok, err, empty, off] {
            assert_eq!(serde_json::from_str::<ToRelay>(&text(&m)).unwrap(), m);
        }
    }

    #[test]
    fn answers_convert_both_ways() {
        let r = Reply::Ok(vec!["a".into()]);
        assert_eq!(ToRelay::answer(1, r.clone()).into_reply(), Some((1, r)));
        let r = Reply::Err("x".into());
        assert_eq!(ToRelay::answer(2, r.clone()).into_reply(), Some((2, r)));
        assert_eq!(
            ToRelay::Bye {
                bye: ByeReason::Quit
            }
            .into_reply(),
            None
        );
        // A failed answer without text is still a failure.
        let odd: ToRelay = serde_json::from_str(r#"{"reply":3,"ok":false}"#).unwrap();
        assert!(matches!(odd.into_reply(), Some((3, Reply::Err(_)))));
    }

    #[test]
    fn malformed_messages_are_errors_not_panics() {
        for bad in [
            "",
            "null",
            "[]",
            "7",
            "\"x\"",
            "{}",
            r#"{"call":1}"#,
            r#"{"call":-1,"tool":"a","args":{}}"#,
            r#"{"cancel":"x"}"#,
            r#"{"bye":"later"}"#,
            "{\"call\":1,",
        ] {
            assert!(serde_json::from_str::<ToApp>(bad).is_err(), "{bad:?}");
            assert!(serde_json::from_str::<ToRelay>(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn debug_never_prints_content() {
        let call = ToApp::Call {
            call: 4,
            tool: "write_file".into(),
            args: json!({"content": "SECRET-BODY", "path": "/x"}),
        };
        let shown = format!("{call:?}");
        assert!(
            !shown.contains("SECRET-BODY") && !shown.contains("/x"),
            "{shown}"
        );
        assert!(shown.contains("write_file") && shown.contains('4'));
        let answer = ToRelay::answer(5, Reply::Ok(vec!["SECRET-OUTPUT".into()]));
        let shown = format!("{answer:?}");
        assert!(!shown.contains("SECRET-OUTPUT"), "{shown}");
        let failed = ToRelay::answer(5, Reply::Err("SECRET-ERR".into()));
        assert!(!format!("{failed:?}").contains("SECRET-ERR"));
        assert!(!format!("{:?}", Reply::Ok(vec!["SECRET".into()])).contains("SECRET"));
        assert!(!format!("{:?}", Reply::Err("SECRET".into())).contains("SECRET"));
    }

    #[test]
    fn hello_rule() {
        let own = env!("CARGO_PKG_VERSION");
        let same = Hello {
            hoprs_agent: PROTOCOL,
            version: own.into(),
        };
        assert_eq!(
            hello_reply(&same),
            HelloReply::Ok {
                version: own.into()
            }
        );
        let number = Hello {
            hoprs_agent: PROTOCOL + 1,
            version: own.into(),
        };
        assert_eq!(
            hello_reply(&number),
            HelloReply::Mismatch {
                version: own.into()
            }
        );
        let version = Hello {
            hoprs_agent: PROTOCOL,
            version: format!("{own}-other"),
        };
        assert_eq!(
            hello_reply(&version),
            HelloReply::Mismatch {
                version: own.into()
            }
        );
    }

    #[tokio::test]
    async fn read_line_limits() {
        let mut two: &[u8] = b"{\"a\":1}\n{\"b\":2}\n";
        assert_eq!(
            read_line(&mut two).await.unwrap().as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(
            read_line(&mut two).await.unwrap().as_deref(),
            Some("{\"b\":2}")
        );
        assert_eq!(read_line(&mut two).await.unwrap(), None);

        // A last line without the newline is returned; a CRLF loses both.
        let mut last: &[u8] = b"x\r\ny";
        assert_eq!(read_line(&mut last).await.unwrap().as_deref(), Some("x"));
        assert_eq!(read_line(&mut last).await.unwrap().as_deref(), Some("y"));
        assert_eq!(read_line(&mut last).await.unwrap(), None);

        // Exactly the limit passes; one byte more is an error.
        let mut exact = vec![b'a'; MAX_LINE];
        exact.push(b'\n');
        let mut r: &[u8] = &exact;
        assert_eq!(read_line(&mut r).await.unwrap().unwrap().len(), MAX_LINE);
        let mut long = vec![b'a'; MAX_LINE + 1];
        long.push(b'\n');
        let mut r: &[u8] = &long;
        assert_eq!(
            read_line(&mut r).await.unwrap_err().kind(),
            ErrorKind::InvalidData
        );
        // ... also without a newline at all (end-of-file inside a too-long line).
        let long = vec![b'a'; MAX_LINE + 1];
        let mut r: &[u8] = &long;
        assert_eq!(
            read_line(&mut r).await.unwrap_err().kind(),
            ErrorKind::InvalidData
        );

        let mut bad: &[u8] = b"\xff\xfe\n";
        assert_eq!(
            read_line(&mut bad).await.unwrap_err().kind(),
            ErrorKind::InvalidData
        );
    }

    #[tokio::test]
    async fn read_line_does_not_buffer_past_the_limit() {
        // A reader that never ends: the error must come after about MAX_LINE bytes.
        let (mut tx, rx) = tokio::io::duplex(64 * 1024);
        let writer = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let chunk = vec![b'a'; 64 * 1024];
            let mut sent = 0usize;
            while tx.write_all(&chunk).await.is_ok() {
                sent += chunk.len();
                if sent > 4 * MAX_LINE {
                    break;
                }
            }
            sent
        });
        let mut reader = tokio::io::BufReader::new(rx);
        let err = read_line(&mut reader).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        drop(reader);
        let sent = writer.await.unwrap();
        assert!(
            sent <= 2 * MAX_LINE,
            "the writer was let through {sent} bytes"
        );
    }

    #[tokio::test]
    async fn write_line_writes_one_flushed_line() {
        let mut out: Vec<u8> = Vec::new();
        write_line(&mut out, &ToApp::Cancel { cancel: 3 })
            .await
            .unwrap();
        write_line(
            &mut out,
            &HelloReply::Ok {
                version: "1".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            b"{\"cancel\":3}\n{\"hello\":\"ok\",\"version\":\"1\"}\n"
        );
        // A text with a newline in it stays one line.
        let mut out: Vec<u8> = Vec::new();
        write_line(
            &mut out,
            &ToRelay::answer(1, Reply::Ok(vec!["a\nb".into()])),
        )
        .await
        .unwrap();
        assert_eq!(out.iter().filter(|b| **b == b'\n').count(), 1);
        // A message over the limit is refused rather than sent.
        let mut out: Vec<u8> = Vec::new();
        let huge = ToRelay::answer(1, Reply::Ok(vec!["a".repeat(MAX_LINE)]));
        let err = write_line(&mut out, &huge).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn the_hello_is_small() {
        let hello = format!(
            r#"{{"hoprs_agent":1,"version":"{}"}}"#,
            env!("CARGO_PKG_VERSION")
        );
        // Padded to exactly the limit (spaces after the object are still JSON): read.
        let fits = format!("{hello:<HELLO_MAX$}\n");
        let mut r = fits.as_bytes();
        assert!(read_hello(&mut r).await.unwrap().is_some());
        // One byte more: refused, though `read_message` would take it.
        let over = format!("{hello:<width$}\n", width = HELLO_MAX + 1);
        let mut r = over.as_bytes();
        assert_eq!(
            read_hello(&mut r).await.unwrap_err().kind(),
            ErrorKind::InvalidData
        );
        let mut r = over.as_bytes();
        assert!(read_message::<_, Hello>(&mut r).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn read_message_wants_one_json_object() {
        let mut r: &[u8] = b"{\"cancel\":1}\n[1]\n\n{\"cancel\":\n";
        assert_eq!(
            read_message::<_, ToApp>(&mut r).await.unwrap(),
            Some(ToApp::Cancel { cancel: 1 })
        );
        for _ in 0..3 {
            let err = read_message::<_, ToApp>(&mut r).await.unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidData);
        }
        assert_eq!(read_message::<_, ToApp>(&mut r).await.unwrap(), None);
    }
}
