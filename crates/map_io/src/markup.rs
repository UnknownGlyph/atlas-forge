use std::borrow::Cow;

/// Escape text and double-quoted attribute values shared by HTML and SVG.
pub(crate) fn escape(text: &str) -> Cow<'_, str> {
    let mut matches = text.match_indices(['&', '<', '>', '"']).peekable();
    if matches.peek().is_none() {
        return Cow::Borrowed(text);
    }
    let mut escaped = String::with_capacity(text.len());
    let mut start = 0;
    for (index, matched) in matches {
        escaped.push_str(&text[start..index]);
        escaped.push_str(match matched {
            "&" => "&amp;",
            "<" => "&lt;",
            ">" => "&gt;",
            "\"" => "&quot;",
            _ => unreachable!(),
        });
        start = index + matched.len();
    }
    escaped.push_str(&text[start..]);
    Cow::Owned(escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup_once_without_touching_unicode_or_apostrophes() {
        assert!(matches!(escape("山岳"), Cow::Borrowed("山岳")));
        assert_eq!(escape("北岸<'A&B'>\"&amp;"), "北岸&lt;'A&amp;B'&gt;&quot;&amp;amp;");
    }
}
