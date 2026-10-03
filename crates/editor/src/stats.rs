pub struct NoteStats {
    pub char_count: i32,
    pub word_count: i32,
    pub line_count: i32,
}

/// Mirrors `tomet_stats::measure`'s character/word counts when `body`
/// parses, falling back to a plain-text approximation while it doesn't
/// (e.g. mid-keystroke on an unfinished `@element(`).
pub fn note_stats(body: &str) -> NoteStats {
    let line_count = if body.is_empty() {
        0
    } else {
        body.lines().count() as i32
    };

    let (char_count, word_count) = match tomet_parser::parse_document(body) {
        Ok(doc) => {
            let stats = tomet_stats::measure(&doc);
            (stats.characters as i32, stats.words as i32)
        }
        Err(_) => {
            let char_count = body.chars().filter(|c| !c.is_whitespace()).count() as i32;
            let word_count = body.split_whitespace().count() as i32;
            (char_count, word_count)
        }
    };

    NoteStats {
        char_count,
        word_count,
        line_count,
    }
}
