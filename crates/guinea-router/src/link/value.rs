//! What a deep link is allowed to carry.
//!
//! A path used to decode with `FromStr` and encode with `Display`, which meant
//! any type in reach could sit in a link. That is too much freedom for the one
//! part of an application that is not the application's to change: once
//! shipped, an address lives in shortcuts, in mails, and in other programs'
//! integrations, and it has to mean the same thing to an installer, to the
//! next version, and to whoever typed it.
//!
//! So the set is closed, and closed with a sealed trait rather than a list of
//! names inside the macro. The macro sees tokens; `type Context = String`
//! would fail a textual check and passes this one, because at the type level
//! it really is a `String`.
//!
//! Closing it also makes encoding fixable. `Display` for a `String` puts `/`
//! and spaces straight into the path, so a capture of `"a/b"` used to build a
//! link that read as two segments. [`LinkValue::encode`] percent-encodes and
//! [`LinkValue::decode`] undoes it; with an open `Display` there was nowhere
//! to put that.
//!
//! The rule binds captures of a route that declared a `link`, and nothing
//! else. A route with no address is ordinary Rust and its fields are whatever
//! the author wants.

mod sealed {
    pub trait Sealed {}
}

/// A value a link may carry.
///
/// Implemented for the standard types that survive a round trip through a path
/// segment, and sealed, so the set is guinea's to widen rather than anyone's
/// to reach into: a type of the application's own cannot be let in, however
/// well it parses.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot appear in a link",
    label = "a link carries only standard types",
    note = "a deep link is an external address: what it carries has to mean the same thing \
            to an installer, to a shortcut, and to the next version of this application",
    note = "strings, integers, `bool` and `char` are the whole set - a route that wants \
            more than that is a route with no `link`"
)]
pub trait LinkValue: sealed::Sealed + Sized {
    /// What the deep-link manifest calls this type.
    ///
    /// Taken from here rather than from how the declaration spelled it, so an
    /// alias renamed in the application does not show up as a changed external
    /// address.
    const NAME: &'static str;

    /// Reads one path segment. `None` means this branch of the match tree was
    /// the wrong one, not that matching failed.
    fn decode(segment: &str) -> Option<Self>;

    /// Writes one path segment, escaped.
    fn encode(&self) -> String;
}

/// RFC 3986 unreserved: everything else is escaped, which is stricter than a
/// path segment strictly requires and leaves nothing to argue about.
fn unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if unreserved(byte) {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(char::from_digit((byte >> 4) as u32, 16).unwrap().to_ascii_uppercase());
            out.push(char::from_digit((byte & 0xf) as u32, 16).unwrap().to_ascii_uppercase());
        }
    }
    out
}

fn unescape(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut at = 0;

    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = segment.get(at + 1..at + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }

    String::from_utf8(out).ok()
}

impl sealed::Sealed for String {}

impl LinkValue for String {
    const NAME: &'static str = "String";

    fn decode(segment: &str) -> Option<Self> {
        unescape(segment)
    }

    fn encode(&self) -> String {
        escape(self)
    }
}

impl sealed::Sealed for char {}

impl LinkValue for char {
    const NAME: &'static str = "char";

    fn decode(segment: &str) -> Option<Self> {
        let text = unescape(segment)?;
        let mut chars = text.chars();
        let first = chars.next()?;
        chars.next().is_none().then_some(first)
    }

    fn encode(&self) -> String {
        escape(self.encode_utf8(&mut [0u8; 4]))
    }
}

impl sealed::Sealed for bool {}

impl LinkValue for bool {
    const NAME: &'static str = "bool";

    fn decode(segment: &str) -> Option<Self> {
        segment.parse().ok()
    }

    fn encode(&self) -> String {
        self.to_string()
    }
}

macro_rules! numbers {
    ($($ty:ty),* $(,)?) => {
        $(
            impl sealed::Sealed for $ty {}

            impl LinkValue for $ty {
                const NAME: &'static str = stringify!($ty);

                fn decode(segment: &str) -> Option<Self> {
                    segment.parse().ok()
                }

                fn encode(&self) -> String {
                    self.to_string()
                }
            }
        )*
    };
}

numbers!(u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_string_with_a_slash_stays_one_segment() {
        // The bug the closed set exists to make fixable: `Display` used to put
        // this straight into the path, and the link read as two segments.
        let encoded = "a/b".to_string().encode();
        assert_eq!(encoded, "a%2Fb");
        assert!(!encoded.contains('/'));
        assert_eq!(String::decode(&encoded).as_deref(), Some("a/b"));
    }

    #[test]
    fn round_trips_what_a_capture_can_hold() {
        for text in ["ubuntu", "Program Files", "ру́сский", "100%", "", "a+b=c"] {
            let encoded = text.to_string().encode();
            assert_eq!(
                String::decode(&encoded).as_deref(),
                Some(text),
                "round trip of {text:?} through {encoded:?}"
            );
        }
    }

    #[test]
    fn a_broken_escape_is_a_branch_that_did_not_match() {
        // Not an error: the walk tries the next branch, exactly as it does for
        // a literal that did not match.
        assert_eq!(String::decode("%zz"), None);
        assert_eq!(String::decode("%A"), None);
        assert_eq!(String::decode("%FF"), None, "not utf-8 on its own");
    }

    #[test]
    fn numbers_and_bools_read_the_way_they_are_written() {
        assert_eq!(u32::decode("42"), Some(42));
        assert_eq!(u32::decode("-1"), None);
        assert_eq!(i64::decode("-1"), Some(-1));
        assert_eq!(bool::decode("true"), Some(true));
        assert_eq!(bool::decode("True"), None);
        assert_eq!(42u32.encode(), "42");
    }

    #[test]
    fn a_char_is_one_character_escaped() {
        assert_eq!(char::decode("a"), Some('a'));
        assert_eq!('/'.encode(), "%2F");
        assert_eq!(char::decode("%2F"), Some('/'));
        assert_eq!(char::decode("ab"), None);
        assert_eq!(char::decode(""), None);
    }

    #[test]
    fn the_manifest_name_is_the_types_own() {
        // Not how the declaration spelled it: `type Context = String` renders
        // as `String`, so renaming an alias is not a changed address.
        assert_eq!(<String as LinkValue>::NAME, "String");
        assert_eq!(<u32 as LinkValue>::NAME, "u32");
    }
}
