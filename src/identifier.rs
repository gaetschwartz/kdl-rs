#[cfg(feature = "span")]
use miette::SourceSpan;
use std::{fmt::Display, str::FromStr};

use crate::{v2_parser, KdlError, KdlValue};

/// Represents a KDL
/// [Identifier](https://github.com/kdl-org/kdl/blob/main/SPEC.md#identifier).
#[derive(Debug, Clone, Eq)]
pub struct KdlIdentifier {
    pub(crate) value: String,
    pub(crate) repr: Option<String>,
    #[cfg(feature = "span")]
    pub(crate) span: SourceSpan,
}

#[cfg(feature = "arbitrary")]
mod arbitrary_impl {
    use super::*;
    use arbitrary::{Arbitrary, Unstructured};

    /// Characters that are disallowed in KDL identifiers (unquoted form)
    const DISALLOWED_IDENT_CHARS: &[char] =
        &['\\', '/', '(', ')', '{', '}', '[', ']', ';', '"', '#', '='];

    /// Unicode whitespace characters that cannot appear in identifiers
    const UNICODE_SPACES: &[char] = &[
        '\u{0009}', '\u{0020}', '\u{00A0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}',
        '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}',
        '\u{200A}', '\u{202F}', '\u{205F}', '\u{3000}',
    ];

    /// Newline characters
    const NEWLINE_CHARS: &[char] = &[
        '\u{000D}', // CR
        '\u{000A}', // LF
        '\u{0085}', // NEL
        '\u{000B}', // VT
        '\u{000C}', // FF
        '\u{2028}', // LS
        '\u{2029}', // PS
    ];

    /// Keywords that cannot be used as bare identifiers
    const KEYWORDS: &[&str] = &["true", "false", "null", "inf", "-inf", "nan"];

    /// Check if a character is disallowed in identifier strings
    fn is_disallowed_ident_char(c: char) -> bool {
        DISALLOWED_IDENT_CHARS.contains(&c)
            || UNICODE_SPACES.contains(&c)
            || NEWLINE_CHARS.contains(&c)
            || is_disallowed_unicode(c)
    }

    /// Check if a character is a disallowed unicode codepoint per spec section 3.19
    fn is_disallowed_unicode(c: char) -> bool {
        matches!(c,
            '\u{0000}'..='\u{0008}'
            | '\u{000E}'..='\u{001F}'
            | '\u{007F}'
            | '\u{200E}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
        )
    }

    /// Characters that are valid for identifier strings (excluding initial position restrictions)
    const VALID_IDENT_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_-+.!@$%^&*:?<>,~`|'0123456789";

    /// Characters that are valid for the first position in an identifier
    /// (excluding digits and sign/dot with special rules)
    const VALID_INITIAL_CHARS: &[u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_!@$%^&*:?<>,~`|'";

    impl<'a> Arbitrary<'a> for KdlIdentifier {
        fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
            // Decide whether to generate a plain identifier or a quoted string
            let use_quoted: bool = u.arbitrary()?;

            let value = if use_quoted {
                // Generate a valid string value (can be anything except disallowed unicode)
                generate_valid_string_value(u)?
            } else {
                // Generate a valid plain identifier
                generate_valid_plain_ident(u)?
            };

            Ok(KdlIdentifier {
                value,
                repr: None, // Let Display compute the appropriate representation
                #[cfg(feature = "span")]
                span: SourceSpan::from(0..0),
            })
        }
    }

    /// Generate a valid plain (unquoted) identifier
    fn generate_valid_plain_ident(u: &mut Unstructured<'_>) -> arbitrary::Result<String> {
        loop {
            let len = u.int_in_range(1..=20)?;
            let mut result = String::with_capacity(len);

            // Choose the type of identifier:
            // 0 = unambiguous (starts with letter/underscore/etc)
            // 1 = signed (starts with + or -)
            // 2 = dotted (starts with optional sign then .)
            let ident_type: u8 = u.int_in_range(0..=2)?;

            match ident_type {
                0 => {
                    // Unambiguous identifier: starts with non-digit, non-sign, non-dot
                    let idx = u.choose_index(VALID_INITIAL_CHARS.len())?;
                    result.push(VALID_INITIAL_CHARS[idx] as char);

                    // Add remaining characters
                    for _ in 1..len {
                        let idx = u.choose_index(VALID_IDENT_CHARS.len())?;
                        result.push(VALID_IDENT_CHARS[idx] as char);
                    }
                }
                1 => {
                    // Signed identifier: starts with + or -
                    let sign = if u.arbitrary()? { '+' } else { '-' };
                    result.push(sign);

                    if len > 1 {
                        // Second char must NOT be a digit or .
                        let idx = u.choose_index(VALID_INITIAL_CHARS.len())?;
                        result.push(VALID_INITIAL_CHARS[idx] as char);

                        // Rest can be any valid identifier char
                        for _ in 2..len {
                            let idx = u.choose_index(VALID_IDENT_CHARS.len())?;
                            result.push(VALID_IDENT_CHARS[idx] as char);
                        }
                    }
                }
                2 => {
                    // Dotted identifier: optional sign then .
                    if u.arbitrary()? {
                        let sign = if u.arbitrary()? { '+' } else { '-' };
                        result.push(sign);
                    }
                    result.push('.');

                    if result.len() < len {
                        // Next char must NOT be a digit
                        let idx = u.choose_index(VALID_INITIAL_CHARS.len())?;
                        result.push(VALID_INITIAL_CHARS[idx] as char);

                        // Rest can be any valid identifier char
                        for _ in (result.len())..len {
                            let idx = u.choose_index(VALID_IDENT_CHARS.len())?;
                            result.push(VALID_IDENT_CHARS[idx] as char);
                        }
                    }
                }
                _ => unreachable!(),
            }

            // Verify the result is not a keyword
            if !KEYWORDS.contains(&result.as_str()) {
                // Verify all characters are valid
                if !result.chars().any(is_disallowed_ident_char) {
                    return Ok(result);
                }
            }
            // If we generated a keyword or invalid identifier, try again
        }
    }

    /// Generate a valid string value (for quoted identifiers)
    fn generate_valid_string_value(u: &mut Unstructured<'_>) -> arbitrary::Result<String> {
        let len = u.int_in_range(0..=30)?;
        let mut result = String::with_capacity(len);

        // Use a mix of safe ASCII and some extended characters
        const SAFE_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-+.!@#$%^&*(){}[]\\/<>?,;:'\" \t";

        for _ in 0..len {
            let idx = u.choose_index(SAFE_CHARS.len())?;
            let c = SAFE_CHARS[idx] as char;
            // Filter out disallowed unicode (though ASCII is generally fine)
            if !is_disallowed_unicode(c) {
                result.push(c);
            }
        }

        Ok(result)
    }
}

impl PartialEq for KdlIdentifier {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.repr == other.repr
        // intentionally omitted: self.span == other.span
    }
}

impl std::hash::Hash for KdlIdentifier {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.value.hash(state);
        self.repr.hash(state);
        // Intentionally omitted: self.span.hash(state);
    }
}

impl KdlIdentifier {
    /// Gets the string value for this identifier.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Sets the string value for this identifier.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
    }

    /// Gets this identifier's span.
    ///
    /// This value will be properly initialized when created via [`crate::KdlDocument::parse`]
    /// but may become invalidated if the document is mutated. We do not currently
    /// guarantee this to yield any particularly consistent results at that point.
    #[cfg(feature = "span")]
    pub fn span(&self) -> SourceSpan {
        self.span
    }

    /// Sets this identifier's span.
    #[cfg(feature = "span")]
    pub fn set_span(&mut self, span: impl Into<SourceSpan>) {
        self.span = span.into();
    }

    /// Gets the custom string representation for this identifier, if any.
    pub fn repr(&self) -> Option<&str> {
        self.repr.as_deref()
    }

    /// Sets a custom string representation for this identifier.
    pub fn set_repr(&mut self, repr: impl Into<String>) {
        self.repr = Some(repr.into());
    }

    /// Length of this identifier when rendered as a string.
    pub fn len(&self) -> usize {
        format!("{self}").len()
    }

    /// Returns true if this identifier is completely empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Resets this identifier to its default representation. It will attempt
    /// to make it an unquoted identifier, and fall back to a string
    /// representation if that would be invalid.
    pub fn clear_format(&mut self) {
        self.repr = None;
    }

    /// Auto-formats this identifier.
    pub fn autoformat(&mut self) {
        self.repr = None;
    }

    /// Parses a string into a entry.
    ///
    /// If the `v1-fallback` feature is enabled, this method will first try to
    /// parse the string as a KDL v2 entry, and, if that fails, it will try
    /// to parse again as a KDL v1 entry. If both fail, only the v2 parse
    /// errors will be returned.
    pub fn parse(s: &str) -> Result<Self, KdlError> {
        #[cfg(not(feature = "v1-fallback"))]
        {
            v2_parser::try_parse(v2_parser::identifier, s)
        }
        #[cfg(feature = "v1-fallback")]
        {
            v2_parser::try_parse(v2_parser::identifier, s)
                .or_else(|e| KdlIdentifier::parse_v1(s).map_err(|_| e))
        }
    }

    /// Parses a KDL v1 string into an entry.
    #[cfg(feature = "v1")]
    pub fn parse_v1(s: &str) -> Result<Self, KdlError> {
        let ret: Result<kdlv1::KdlIdentifier, kdlv1::KdlError> = s.parse();
        ret.map(|x| x.into()).map_err(|e| e.into())
    }
}

#[cfg(feature = "v1")]
impl From<kdlv1::KdlIdentifier> for KdlIdentifier {
    fn from(value: kdlv1::KdlIdentifier) -> Self {
        Self {
            value: value.value().into(),
            repr: value.repr().map(|x| x.into()),
            #[cfg(feature = "span")]
            span: (value.span().offset(), value.span().len()).into(),
        }
    }
}

impl Display for KdlIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(repr) = &self.repr {
            write!(f, "{repr}")
        } else {
            write!(f, "{}", KdlValue::String(self.value().into()))
        }
    }
}

impl From<&str> for KdlIdentifier {
    fn from(value: &str) -> Self {
        Self {
            value: value.to_string(),
            repr: None,
            #[cfg(feature = "span")]
            span: SourceSpan::from(0..0),
        }
    }
}

impl From<String> for KdlIdentifier {
    fn from(value: String) -> Self {
        Self {
            value,
            repr: None,
            #[cfg(feature = "span")]
            span: SourceSpan::from(0..0),
        }
    }
}

impl From<KdlIdentifier> for String {
    fn from(value: KdlIdentifier) -> Self {
        value.value
    }
}

impl FromStr for KdlIdentifier {
    type Err = KdlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parsing() -> miette::Result<()> {
        let plain = "foo";
        assert_eq!(
            plain.parse::<KdlIdentifier>()?,
            KdlIdentifier {
                value: plain.to_string(),
                repr: Some(plain.to_string()),
                #[cfg(feature = "span")]
                span: SourceSpan::from(0..3),
            }
        );

        let quoted = r#""foo\"bar""#;
        assert_eq!(
            quoted.parse::<KdlIdentifier>()?,
            KdlIdentifier {
                value: "foo\"bar".to_string(),
                repr: Some(quoted.to_string()),
                #[cfg(feature = "span")]
                span: SourceSpan::from(0..0),
            }
        );

        let invalid = "123";
        assert!(invalid.parse::<KdlIdentifier>().is_err());

        let invalid = "   space   ";
        assert!(invalid.parse::<KdlIdentifier>().is_err());

        let invalid = "\"x";
        assert!(invalid.parse::<KdlIdentifier>().is_err());

        Ok(())
    }

    #[test]
    fn formatting() {
        let plain = KdlIdentifier::from("foo");
        assert_eq!(format!("{plain}"), "foo");

        let quoted = KdlIdentifier::from("foo\"bar");
        assert_eq!(format!("{quoted}"), r#""foo\"bar""#);

        let mut custom_repr = KdlIdentifier::from("foo");
        custom_repr.set_repr(r#""foo/bar""#.to_string());
        assert_eq!(format!("{custom_repr}"), r#""foo/bar""#);
    }
}
