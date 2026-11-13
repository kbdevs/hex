//! Transcript preparation and rewrite prompt formatting.

use super::{DictationLabs, PROTOCOL_PROMPT, Profile, Source};
use crate::dictation::context::ContextSnapshot;
use crate::dictation::vocabulary::VocabularyEntry;

/// Whether a transcript says nothing but numbers: digits or English number
/// words, with punctuation between them.
pub(super) fn numbers_only(transcript: &str) -> bool {
    const WORDS: &[&str] = &[
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
        "thirty",
        "forty",
        "fifty",
        "sixty",
        "seventy",
        "eighty",
        "ninety",
        "hundred",
        "thousand",
        "million",
        "billion",
    ];
    /// Words that join numbers but are not numbers themselves.
    const JOINERS: &[&str] = &["and", "point", "minus", "negative"];
    let mut any_number = false;
    for token in transcript.split(|c: char| c.is_whitespace() || c == '-') {
        let word = token.trim_matches(|c: char| !c.is_alphanumeric());
        if word.is_empty() {
            continue;
        }
        if word
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
            || WORDS.iter().any(|w| word.eq_ignore_ascii_case(w))
        {
            any_number = true;
        } else if !JOINERS.iter().any(|w| word.eq_ignore_ascii_case(w)) {
            return false;
        }
    }
    any_number
}

/// The transcript ahead of mode corrections. Spoken formatting, spoken
/// symbols, and filler words only apply to heard speech, so typed text skips
/// every one of them.
pub(super) fn prepare_transcript(
    transcript: &str,
    language: &str,
    source: Source,
    labs: DictationLabs,
) -> String {
    if source != Source::Dictation {
        return transcript.to_owned();
    }
    let formatted;
    let transcript = if labs.spoken_formatting {
        formatted = crate::dictation::spoken_formatting::apply(transcript);
        formatted.as_str()
    } else {
        transcript
    };
    let symbolized;
    let transcript = if labs.spoken_symbols {
        symbolized = crate::dictation::spoken_symbols::apply(transcript);
        symbolized.as_str()
    } else {
        transcript
    };
    // Only English dictations drop fillers: "er" and "um" are ordinary words
    // in other languages, and "auto" says nothing about the language.
    crate::dictation::filler_words::strip_if(transcript, language, labs.filler_words)
        .unwrap_or_else(|| transcript.to_owned())
}

pub(super) fn prompt(
    profile: &Profile,
    transcript: &str,
    context: &ContextSnapshot,
    vocabulary: &[VocabularyEntry],
) -> String {
    let application = context.application.as_deref().unwrap_or("unknown");
    let host = context.browser_host().unwrap_or("none");
    format!(
        "{PROTOCOL_PROMPT}\n\nProfile instructions:\n{}\n\n{}Foreground application: {application}\nBrowser host: {host}\n\nRaw transcript:\n<transcript>\n{transcript}\n</transcript>",
        profile.prompt,
        vocabulary_lines(vocabulary)
    )
}

/// The vocabulary as prompt lines, so a rewrite spells its terms right;
/// nothing when it is empty (or its experiment is off).
pub(super) fn vocabulary_lines(vocabulary: &[VocabularyEntry]) -> String {
    if vocabulary.is_empty() {
        return String::new();
    }
    let terms = vocabulary
        .iter()
        .map(|entry| match entry.sounds_like.as_slice() {
            [] => entry.term.clone(),
            spellings => format!(
                "{} (may be transcribed as {})",
                entry.term,
                spellings.join(", ")
            ),
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("Vocabulary (spell these exactly when the speaker means them): {terms}\n\n")
}
