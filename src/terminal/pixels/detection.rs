//! Bounded synchronous capability query. ratatui-image's convenience query spawns a reader that can outlive its
//! timeout and consume a subsequent key. This query finishes before crossterm starts reading events.
use ratatui_image::{FontSize, picker::ProtocolType};
use std::io;

pub(super) fn detect_protocol(tmux: bool, out: &mut impl io::Write) -> io::Result<(ProtocolType, Option<FontSize>)> {
    let hint = std::env::var("TERM_PROGRAM").unwrap_or_default();
    let prefer_iterm = hint.eq_ignore_ascii_case("rio")
        || hint == "iTerm.app"
        || hint == "WezTerm"
        || std::env::var_os("WEZTERM_EXECUTABLE").is_some()
        || std::env::var_os("KONSOLE_VERSION").is_some();
    let (detected, font) = query_capabilities(tmux, out, prefer_iterm)?;
    Ok((
        if prefer_iterm {
            ProtocolType::Iterm2
        } else {
            detected.unwrap_or(ProtocolType::Halfblocks)
        },
        font,
    ))
}

#[cfg(unix)]
fn query_capabilities(
    tmux: bool,
    out: &mut impl io::Write,
    blacklist: bool,
) -> io::Result<(Option<ProtocolType>, Option<FontSize>)> {
    use ratatui_image::picker::cap_parser::{Parser, QueryStdioOptions, Response};
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use std::time::{Duration, Instant};
    let options = QueryStdioOptions {
        blacklist_protocols: if blacklist {
            vec![ProtocolType::Kitty, ProtocolType::Sixel]
        } else {
            Vec::new()
        },
        ..QueryStdioOptions::default()
    };
    out.write_all(Parser::query(tmux, options).as_bytes())?;
    out.flush()?;
    let deadline = Instant::now() + Duration::from_millis(1500);
    let stdin = io::stdin();
    let mut parser = Parser::new();
    let (mut protocol, mut font) = (None, None);
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
        for byte in &bytes[..count] {
            for response in parser.push(char::from(*byte)) {
                match response {
                    Response::Kitty => protocol = Some(ProtocolType::Kitty),
                    Response::Sixel if protocol.is_none() => protocol = Some(ProtocolType::Sixel),
                    Response::CellSize(Some((w, h))) if w > 0 && h > 0 => font = Some(FontSize::new(w, h)),
                    Response::Status => return Ok((protocol, font)),
                    _ => {}
                }
            }
        }
    }
    Ok((protocol, font))
}

#[cfg(not(unix))]
fn query_capabilities(
    _tmux: bool,
    _out: &mut impl io::Write,
    _blacklist: bool,
) -> io::Result<(Option<ProtocolType>, Option<FontSize>)> {
    Ok((None, None)) // explicit protocols remain available; Windows console queries require separate qualification
}
