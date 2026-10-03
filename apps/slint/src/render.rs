//! Converts `immermemo_editor::classify_body`'s plain-data blocks into the
//! Slint-generated `RenderedBlock` model `editor.slint`'s view-mode
//! display draws (see `crates/editor-slint/ui/rendered_block.slint`'s
//! `RenderedBlockView`). The classification decision -- including which
//! identity looks like what -- lives in `immermemo-editor`
//! (`crates/editor/src/classify.rs`'s `REGISTRY`), which knows nothing
//! about Slint; this module is just the seam between that and the
//! generated `ModelRc<RenderedBlock>` type. It has to live here rather
//! than in `crates/editor-slint` because `RenderedBlock` is only a real
//! Rust type inside whichever crate's `slint::include_modules!()` call
//! compiled a tree that imports it -- today that's `apps/slint` alone
//! (see `crates/editor-slint/src/lib.rs`'s doc comment).

use immermemo_editor::{BlockShape, Tone, classify_body};
use slint::{ModelRc, VecModel};

use crate::{RenderedBlock, RenderedBlockShape, RenderedBlockTone};

/// Classifies `body` and converts the result into the model
/// `App`/`EditorScreen`'s `rendered-blocks` property expects. Callers
/// should set this alongside `body` every time `body` changes -- see
/// `session::update_rendered_body` and `lib.rs`'s `on_edited` handler,
/// the two places that happens.
pub fn rendered_blocks(body: &str) -> ModelRc<RenderedBlock> {
    let blocks: Vec<RenderedBlock> = classify_body(body)
        .into_iter()
        .map(|block| RenderedBlock {
            shape: match block.shape {
                BlockShape::PlainText => RenderedBlockShape::PlainText,
                BlockShape::Chip => RenderedBlockShape::Chip,
                BlockShape::Divider => RenderedBlockShape::Divider,
                BlockShape::Badge => RenderedBlockShape::Badge,
            },
            tone: match block.tone {
                Tone::Accent => RenderedBlockTone::Accent,
                Tone::Warning => RenderedBlockTone::Warning,
                Tone::Neutral => RenderedBlockTone::Neutral,
            },
            text: block.text.into(),
        })
        .collect();
    ModelRc::new(VecModel::from(blocks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    // The substantial classification-decision tests (conflict markers,
    // unrecognized elements, the real merge pipeline) live in
    // `immermemo-editor`'s own tests now. This just checks the seam: that
    // converting into the generated `RenderedBlock` model actually
    // round-trips shape/tone/text correctly.
    #[test]
    fn classified_blocks_convert_into_the_generated_model() {
        let model = rendered_blocks("Before.\n\n@mobile.conflict(mine)\n\nMine.\n");
        assert_eq!(model.row_count(), 3);
        assert_eq!(
            model.row_data(0).unwrap().shape,
            RenderedBlockShape::PlainText
        );
        assert_eq!(model.row_data(0).unwrap().text, "Before.");
        assert_eq!(model.row_data(1).unwrap().shape, RenderedBlockShape::Chip);
        assert_eq!(model.row_data(1).unwrap().tone, RenderedBlockTone::Accent);
        assert_eq!(model.row_data(1).unwrap().text, "Mine");
        assert_eq!(
            model.row_data(2).unwrap().shape,
            RenderedBlockShape::PlainText
        );
    }
}
