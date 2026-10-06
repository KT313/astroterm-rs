//! Bounded synchronous capability query, completed before crossterm starts reading keys. Forced Kitty mode
//! still probes compression: support for uncompressed images does not imply support for zlib transfers.
use ratatui_image::{
    FontSize,
    picker::{
        ProtocolType,
        cap_parser::{Parser, QueryStdioOptions, Response},
    },
};
use std::io;

use crate::state::CompressionSupport;
pub(super) fn describe_compression(value: CompressionSupport) -> &'static str {
    match value {
        CompressionSupport::Supported => "Enabled (zlib)",
        CompressionSupport::Unsupported => "Not supported by terminal; using RGB",
        CompressionSupport::Unknown => "Support not confirmed; using RGB",
    }
}

#[derive(Default)]
struct Capabilities {
    protocol: Option<ProtocolType>,
    font: Option<FontSize>,
    compression: CompressionSupport,
}

pub(super) fn detect_protocol(
    forced: Option<ProtocolType>,
    tmux: bool,
    out: &mut impl io::Write,
) -> io::Result<(ProtocolType, Option<FontSize>, CompressionSupport)> {
    if let Some(protocol) = forced.filter(|p| *p != ProtocolType::Kitty) {
        return Ok((protocol, None, CompressionSupport::Unknown));
    }
    let hint = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let prefer_iterm = forced.is_none()
        && (hint.eq_ignore_ascii_case("rio")
            || hint == "iTerm.app"
            || hint == "WezTerm"
            || std::env::var_os("WEZTERM_EXECUTABLE").is_some()
            || std::env::var_os("KONSOLE_VERSION").is_some());
    let capabilities = query_capabilities(tmux, out, prefer_iterm)?;
    let protocol = forced.unwrap_or_else(|| {
        if prefer_iterm {
            ProtocolType::Iterm2
        } else {
            capabilities.protocol.unwrap_or(ProtocolType::Halfblocks)
        }
    });
    Ok((protocol, capabilities.font, capabilities.compression))
}

#[derive(Default)]
struct Replies {
    parser: Parser,
    tail: String,
    capabilities: Capabilities,
}

impl Replies {
    fn push(&mut self, byte: u8) -> bool {
        // The library reports successful compression probes but discards explicit rejection replies.
        self.tail.push(char::from(byte));
        if self.tail.ends_with("\x1b\\") {
            if let Some(start) = self.tail.rfind("\x1b_Gi=32;") {
                let message = &self.tail[start + "\x1b_Gi=32;".len()..self.tail.len() - 2];
                self.capabilities.compression = if message == "OK" {
                    CompressionSupport::Supported
                } else {
                    CompressionSupport::Unsupported
                };
            }
            self.tail.clear();
        } else if self.tail.len() > 1024 {
            self.tail.clear(); // bounded storage even for malformed terminal replies
        }

        let mut complete = false;
        for response in self.parser.push(char::from(byte)) {
            match response {
                Response::Kitty => self.capabilities.protocol = Some(ProtocolType::Kitty),
                Response::KittyCompression => {
                    self.capabilities.protocol = Some(ProtocolType::Kitty);
                    self.capabilities.compression = CompressionSupport::Supported;
                }
                Response::Sixel if self.capabilities.protocol.is_none() => {
                    self.capabilities.protocol = Some(ProtocolType::Sixel)
                }
                Response::CellSize(Some((w, h))) if w > 0 && h > 0 => {
                    self.capabilities.font = Some(FontSize::new(w, h))
                }
                Response::Status => complete = true,
                _ => {}
            }
        }
        complete
    }
}

fn build_query(tmux: bool, blacklist: bool) -> String {
    Parser::query(
        tmux,
        QueryStdioOptions {
            blacklist_protocols: if blacklist {
                vec![ProtocolType::Kitty, ProtocolType::Sixel]
            } else {
                Vec::new()
            },
            kitty_compression: !blacklist,
            ..QueryStdioOptions::default()
        },
    )
}

#[cfg(unix)]
fn query_capabilities(tmux: bool, out: &mut impl io::Write, blacklist: bool) -> io::Result<Capabilities> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use std::time::{Duration, Instant};
    out.write_all(build_query(tmux, blacklist).as_bytes())?;
    out.flush()?;
    let deadline = Instant::now() + Duration::from_millis(1500);
    let stdin = io::stdin();
    let mut replies = Replies::default();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let timeout = Timespec::try_from(remaining).map_err(io::Error::other)?;
        let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => break,
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        let mut bytes = [0; 128];
        let count = rustix::io::read(&stdin, &mut bytes)?;
        if count == 0 {
            break;
        }
        let mut complete = false;
        for byte in &bytes[..count] {
            complete |= replies.push(*byte);
        }
        if complete {
            break;
        }
    }
    Ok(replies.capabilities)
}

#[cfg(not(unix))]
fn query_capabilities(_tmux: bool, _out: &mut impl io::Write, _blacklist: bool) -> io::Result<Capabilities> {
    Ok(Capabilities::default()) // no compression without confirmation; Windows queries need separate qualification
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compression_query_and_replies_distinguish_support_rejection_and_silence() {
        assert!(build_query(false, false).contains("i=32,s=1,v=1,a=q,t=d,f=24,o=z;"));
        assert!(!build_query(false, true).contains("i=32"));
        assert!(build_query(true, false).starts_with("\x1bPtmux;"));
        for (reply, expected) in [
            ("\x1b_Gi=32;OK\x1b\\", CompressionSupport::Supported),
            (
                "\x1b_Gi=32;ENOTSUP:compressed payloads are not supported\x1b\\",
                CompressionSupport::Unsupported,
            ),
            ("", CompressionSupport::Unknown),
        ] {
            let mut replies = Replies::default();
            let input = format!("\x1b_Gi=31;OK\x1b\\{reply}\x1b[6;20;10t\x1b[0n");
            let mut complete = false;
            for byte in input.bytes() {
                complete |= replies.push(byte);
            }
            assert!(complete);
            assert_eq!(replies.capabilities.protocol, Some(ProtocolType::Kitty));
            assert_eq!(replies.capabilities.compression, expected);
            let font = replies.capabilities.font.unwrap();
            assert_eq!((font.width, font.height), (10, 20));
        }
        assert!(
            describe_compression(CompressionSupport::Unsupported)
                .contains("Not supported by terminal")
        );
        assert!(describe_compression(CompressionSupport::Unknown).contains("not confirmed"));
    }
}
