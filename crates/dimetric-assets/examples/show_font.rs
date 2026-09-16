//! Print the built-in font, so an edit to a glyph can be checked by eye.
//!
//! The glyphs in `builtin_font.rs` are drawn by hand as rows of `#` and `.`,
//! which makes a change reviewable in a diff but does not tell you whether the
//! letter still reads. Run this after editing one:
//!
//! ```text
//! cargo run -p dimetric-assets --example show_font
//! ```

fn main() {
    let (font, page) = dimetric_assets::builtin_font::builtin();
    // A pangram-ish sample: every shape that is easy to get wrong, plus the
    // descenders and the digits a debug overlay is mostly made of.
    for line in [
        "Hamburgefonstiv 0123456789",
        "The quick brown fox; jumps?",
        "{[(<=+-*/\\>)]} @#$%^&_~`\"'!,.:|",
    ] {
        let mut rows = vec![String::new(); dimetric_assets::builtin_font::GLYPH_HEIGHT as usize];
        for c in line.chars() {
            let Some(g) = font.glyphs.get(&c) else {
                continue;
            };
            for (y, row) in rows.iter_mut().enumerate() {
                for x in 0..dimetric_assets::builtin_font::GLYPH_WIDTH {
                    let lit = g.width > 0 && {
                        let px = ((y as u32) * page.width + g.x + x) as usize;
                        page.pixels[px * 4 + 3] > 0
                    };
                    row.push(if lit { '#' } else { ' ' });
                }
                row.push(' ');
            }
        }
        for r in rows {
            println!("{r}");
        }
        println!();
    }
}
