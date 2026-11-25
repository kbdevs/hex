import Foundation

/// A suggestion picked before a space or closing mark defers its automatic space.
/// The next word gets that space, and punctuation can go directly after the suggestion.
@MainActor
extension FullKeyboardEngine {
    /// Marks a space never goes before: closing punctuation, quotes and apostrophes.
    static let closingMarks: Set<Character> = [".", ",", "!", "?", ";", ":", ")", "]", "}", "\"", "'", "»", "…"]
    /// French puts a space before these (`FrenchSpacing`), so there they take it as usual.
    static let frenchSpacedMarks: Set<Character> = ["?", "!", ";", ":", "»"]

    /// Whether the text after the cursor starts with what a space after the word before it
    /// would double: a space (not a line break) or a closing mark.
    var textAfterSeparates: Bool {
        guard let first = target.documentContextAfterInput.map(SmartPunctuation.plain)?.first else { return false }
        if first.isWhitespace { return !first.isNewline }
        guard Self.closingMarks.contains(first) else { return false }
        let french = [KeyboardLanguage.french, .frenchCanadian, .frenchSwiss].contains(layout.language)
        return !(french && Self.frenchSpacedMarks.contains(first))
    }

    /// Whether a held space waits for the next key: nothing has changed since it was held.
    var spacePending: Bool {
        guard let held = suggestionSpaceBefore, held.last?.isWhitespace == false else { return false }
        return held == typingContext && textAfterSeparates
    }

    /// The text before the cursor as the strip reads it: with a held space, as after a typed
    /// one, so the strip offers the next word rather than the word just finished.
    var stripBefore: String? {
        spacePending ? textBefore.map { $0 + " " } : textBefore
    }

    /// Types the held space before a pick from the strip, which then offers the next word:
    /// the pick goes in after it, as after a typed space, never in the finished word's place.
    func typeHeldSpace(before suggestion: KeyboardSuggestion) {
        guard spacePending, suggestion.kind != .revert, suggestions.contains(suggestion) else { return }
        suggestionSpaceBefore = nil
        typingLog?.edit("insert", [
            TypingLogField("inserted", .string(" ")),
            TypingLogField("src", .string("held_space")),
        ])
        rememberTypingContext(before: typingContext, inserted: " ")
        target.insertText(" ")
    }

    /// Types a picked suggestion's word and the space after it, held where the text after the
    /// cursor separates it already (`textAfterSeparates`).
    func insertPicked(_ word: String) {
        let holds = textAfterSeparates
        insert(holds ? word : word + " ", source: nil)
        suggestionSpaceBefore = typingContext
        if holds { refreshSuggestions() }
        typingLog?.context("suggestion")
    }

    /// Types the held space before `text`, the next key, when it starts another word (anything
    /// but a mark in `swipeJoiners`); else a swiped word's (`spaceAfterSwipedWord`).
    func spaceBeforeNextWord(_ text: String) {
        guard spacePending, let first = text.first, !Self.swipeJoiners.contains(first) else {
            return spaceAfterSwipedWord(before: text)
        }
        suggestionSpaceBefore = nil
        typingLog?.edit("insert", [
            TypingLogField("inserted", .string(" ")),
            TypingLogField("src", .string("held_space")),
        ])
        rememberTypingContext(before: typingContext, inserted: " ")
        target.insertText(" ")
        if shift.state == .off { refreshAutoCapitalization() }
    }

    /// Delete right after a held space takes it back and deletes nothing. True when it did.
    func dropsHeldSpace() -> Bool {
        guard spacePending else { return false }
        suggestionSpaceBefore = nil
        typingLog?.edit("delete", [
            TypingLogField("deletedCount", .int(0)),
            TypingLogField("src", .string("held_space")),
        ])
        refreshSuggestions()
        onChange?()
        return true
    }
}
