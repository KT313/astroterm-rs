//! Bounded cell formatting. Collections stop before visiting hidden elements; strings stop before escaping text.
use crate::constants::{MAX_CELL_CHARS, MAX_NESTED_ITEMS, MAX_PREVIEW_DEPTH};
use std::fmt::{self, Write};


/// Types explicitly supported by table previews. No blanket Debug implementation expands arbitrary containers.
pub trait Preview {
    fn write_preview(&self, out: &mut CellWriter, depth: usize) -> fmt::Result;
}

/// A small formatting sink. Errors stop formatting when the cell is full; they are not application I/O errors.
pub struct CellWriter { text: String, remaining: usize, truncated: bool }
impl CellWriter {
    fn new() -> Self { Self { text: String::new(), remaining: MAX_CELL_CHARS, truncated: false } }
    fn finish(mut self) -> String {
        if self.truncated { self.text.push('…'); }
        self.text
    }
}
impl Write for CellWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for ch in text.chars() {
            if self.remaining == 0 { self.truncated = true; return Err(fmt::Error); }
            if ch.is_control() {
                for escaped in ch.escape_default() {
                    if self.remaining == 0 { self.truncated = true; return Err(fmt::Error); }
                    self.text.push(escaped);
                    self.remaining -= 1;
                }
            } else {
                self.text.push(ch);
                self.remaining -= 1;
            }
        }
        Ok(())
    }
}

pub fn preview(value: &(impl Preview + ?Sized)) -> String {
    let mut out = CellWriter::new();
    let _ = value.write_preview(&mut out, 0);
    out.finish()
}

/// Format only a bounded prefix of a text row, without allocating the rest of the row.
pub fn preview_text<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = CellWriter::new();
    for part in parts { if out.write_str(part).is_err() { break; } }
    out.finish()
}
pub fn preview_chars(chars: impl IntoIterator<Item = char>) -> String {
    let mut out = CellWriter::new();
    for ch in chars { if out.write_char(ch).is_err() { break; } }
    out.finish()
}

/// Register Debug only for fixed-size records/enums whose formatter cannot expand owned containers.
macro_rules! debug_preview {
    ($($ty:ty),+ $(,)?) => { $(
        impl $crate::rows::Preview for $ty {
            fn write_preview(&self, out: &mut $crate::rows::CellWriter, _: usize) -> ::std::fmt::Result {
                ::std::fmt::Write::write_fmt(out, format_args!("{self:?}"))
            }
        }
    )+ };
}
pub(crate) use debug_preview;
debug_preview!(bool, char, u8, u16, u32, u64, usize, i32, i64, f32, f64);

impl Preview for str {
    fn write_preview(&self, out: &mut CellWriter, _: usize) -> fmt::Result {
        let cut = self.char_indices().nth(MAX_CELL_CHARS).map_or(self.len(), |(i, _)| i);
        write!(out, "{:?}", &self[..cut])?;
        if cut < self.len() { out.write_str("…")?; }
        Ok(())
    }
}
impl Preview for String {
    fn write_preview(&self, out: &mut CellWriter, depth: usize) -> fmt::Result { self.as_str().write_preview(out, depth) }
}
impl<T: Preview + ?Sized> Preview for &T {
    fn write_preview(&self, out: &mut CellWriter, depth: usize) -> fmt::Result { (**self).write_preview(out, depth) }
}
impl<T: Preview> Preview for Option<T> {
    fn write_preview(&self, out: &mut CellWriter, depth: usize) -> fmt::Result {
        match self {
            None => out.write_str("None"),
            Some(value) => {
                if depth >= MAX_PREVIEW_DEPTH { return out.write_str("Some(…)"); }
                out.write_str("Some(")?; value.write_preview(out, depth + 1)?; out.write_str(")")
            }
        }
    }
}
impl<T: Preview> Preview for [T] {
    fn write_preview(&self, out: &mut CellWriter, depth: usize) -> fmt::Result {
        if depth >= MAX_PREVIEW_DEPTH { return write!(out, "[{} items; …]", self.len()); }
        if self.len() > MAX_NESTED_ITEMS { write!(out, "len={} ", self.len())?; }
        out.write_char('[')?;
        for (i, item) in self.iter().take(MAX_NESTED_ITEMS).enumerate() {
            if i != 0 { out.write_str(", ")?; }
            item.write_preview(out, depth + 1)?;
        }
        if self.len() > MAX_NESTED_ITEMS { write!(out, ", … {} omitted", self.len() - MAX_NESTED_ITEMS)?; }
        out.write_char(']')
    }
}
impl<T: Preview> Preview for Vec<T> {
    fn write_preview(&self, out: &mut CellWriter, depth: usize) -> fmt::Result { self.as_slice().write_preview(out, depth) }
}
impl<T: Preview, const N: usize> Preview for [T; N] {
    fn write_preview(&self, out: &mut CellWriter, depth: usize) -> fmt::Result { self.as_slice().write_preview(out, depth) }
}
macro_rules! tuple_preview {
    ($(($($t:ident),+)),+) => { $(
        #[allow(non_snake_case)]
        impl<$($t: Preview),+> Preview for ($($t,)+) {
            fn write_preview(&self, out: &mut CellWriter, depth: usize) -> fmt::Result {
                if depth >= MAX_PREVIEW_DEPTH { return out.write_str("(…)"); }
                let ($($t,)+) = self;
                out.write_char('(')?;
                let mut separator = "";
                $(out.write_str(separator)?; $t.write_preview(out, depth + 1)?; separator = ", ";)+
                let _ = separator;
                out.write_char(')')
            }
        }
    )+ };
}
tuple_preview!((A, B), (A, B, C), (A, B, C, D));

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Counted<'a>(&'a Cell<usize>);
    impl Preview for Counted<'_> {
        fn write_preview(&self, out: &mut CellWriter, _: usize) -> fmt::Result {
            self.0.set(self.0.get() + 1);
            out.write_char('x')
        }
    }
    #[test]
    fn large_collections_do_not_visit_hidden_elements() {
        let visits = Cell::new(0);
        let values: Vec<_> = (0..100_000).map(|_| Counted(&visits)).collect();
        let text = preview(&values);
        assert_eq!(visits.get(), MAX_NESTED_ITEMS);
        assert!(text.contains("len=100000") && text.contains("99996 omitted"));
        assert!(text.chars().count() <= MAX_CELL_CHARS + 1);
    }
    #[test]
    fn nested_depth_stops_before_the_leaf_formatter() {
        let visits = Cell::new(0);
        let value = vec![vec![vec![vec![Counted(&visits)]]]];
        assert!(preview(&value).contains("1 items; …"));
        assert_eq!(visits.get(), 0);
    }
    #[test]
    fn strings_unicode_and_control_characters_have_bounded_output() {
        let text = preview(&"é".repeat(1_000_000));
        assert!(text.ends_with('…'));
        assert!(text.chars().count() <= MAX_CELL_CHARS + 1);
        let escaped = preview_text(["a\n\r\t\x1b | b"]);
        assert_eq!(escaped, "a\\n\\r\\t\\u{1b} | b");
        assert!(!escaped.chars().any(char::is_control));
        let mut visits = 0;
        let text = preview_chars((0..1_000_000).map(|_| { visits += 1; 'x' }));
        assert_eq!(visits, MAX_CELL_CHARS + 1);
        assert!(text.ends_with('…'));
    }
}
