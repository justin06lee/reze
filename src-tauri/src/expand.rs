//! Expand-in-place: work out which trigger the user just typed.
//!
//! There is no way to ask another application "what are the last few words
//! before the caret", so we select them, copy them, and read them back. This
//! module owns the pure part of that — deciding what the copied text means.

/// Never select more than this, however long a macro name gets. Each extra
/// word is another synthesized keystroke in someone else's text field.
const WORD_CEILING: usize = 8;

/// How many words to select before reading, given the macro names on offer.
///
/// Selecting exactly as many words as the longest trigger means one copy
/// round-trip instead of one per word.
pub fn words_to_select<S: AsRef<str>>(names: &[S]) -> usize {
    names
        .iter()
        .map(|n| n.as_ref().split_whitespace().count())
        .max()
        .unwrap_or(1)
        .clamp(1, WORD_CEILING)
}

/// Words in `text`, each with the byte offset it starts at.
fn words_with_offsets(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                out.push((s, &text[s..i]));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push((s, &text[s..]));
    }
    out
}

/// The longest trigger that matches a whole-word suffix of `text`.
///
/// Returns the index of the matching name plus the byte range the trigger
/// occupies. The caller replaces the *whole* selection with
/// `text[..start] + expansion + text[end..]`, which keeps the surrounding
/// characters byte-for-byte — trying to shrink the selection with word-motion
/// keystrokes instead eats the space before the trigger, because Option+Right
/// lands at the end of a word rather than the start of the next.
///
/// Longest wins: with both `analysis` and `full analysis` defined, typing
/// "full analysis" must expand the latter.
pub fn match_trigger<S: AsRef<str>>(text: &str, names: &[S]) -> Option<(usize, usize, usize)> {
    let words = words_with_offsets(text);
    if words.is_empty() {
        return None;
    }

    for span in (1..=words.len()).rev() {
        let first = words.len() - span;
        let candidate = words[first..]
            .iter()
            .map(|(_, w)| *w)
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();

        for (i, name) in names.iter().enumerate() {
            let name = name.as_ref().trim().to_lowercase();
            if !name.is_empty() && name == candidate {
                let (start, _) = words[first];
                let (last_start, last_word) = words[words.len() - 1];
                return Some((i, start, last_start + last_word.len()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 4] = ["analysis", "full analysis", "commit message", "rigor"];

    /// (matched name, text kept before, the trigger itself, text kept after)
    fn split(text: &str) -> Option<(&str, &str, &str, &str)> {
        let (i, start, end) = match_trigger(text, &NAMES)?;
        Some((NAMES[i], &text[..start], &text[start..end], &text[end..]))
    }

    #[test]
    fn matches_the_longest_trigger_not_the_first() {
        assert_eq!(
            split("please do a full analysis"),
            Some(("full analysis", "please do a ", "full analysis", "")),
        );
    }

    #[test]
    fn keeps_the_space_before_the_trigger() {
        // Regression: shrinking the selection by word motions used to swallow
        // this space, gluing the expansion onto the previous word.
        let (_, head, _, _) = split("please do a full analysis").unwrap();
        assert!(head.ends_with(' '), "head was {head:?}");
    }

    #[test]
    fn matches_a_single_word_trigger() {
        assert_eq!(split("some analysis"), Some(("analysis", "some ", "analysis", "")));
    }

    #[test]
    fn is_case_insensitive_and_preserves_surrounding_whitespace() {
        assert_eq!(
            split("  Commit   Message  "),
            Some(("commit message", "  ", "Commit   Message", "  ")),
        );
    }

    #[test]
    fn only_matches_at_the_end() {
        // The caret is after "now", so "rigor" is not what was just typed.
        assert_eq!(split("rigor now"), None);
    }

    #[test]
    fn rejects_partial_words() {
        assert_eq!(split("rigorous"), None);
    }

    #[test]
    fn handles_empty_input() {
        assert_eq!(split("   "), None);
        assert_eq!(split(""), None);
    }

    #[test]
    fn handles_multibyte_text_before_the_trigger() {
        // Byte offsets, not char offsets — slicing must land on a boundary.
        let text = "héllo — full analysis";
        let (_, head, trigger, tail) = split(text).unwrap();
        assert_eq!(trigger, "full analysis");
        assert_eq!(head, "héllo — ");
        assert_eq!(tail, "");
    }

    #[test]
    fn selection_width_follows_the_longest_name() {
        assert_eq!(words_to_select(&NAMES), 2);
        assert_eq!(words_to_select(&["a b c d e f g h i j"]), WORD_CEILING);
        assert_eq!(words_to_select::<&str>(&[]), 1);
    }

    #[test]
    fn ignores_blank_names() {
        assert_eq!(match_trigger("something", &["", "   "]), None);
    }
}
