//! The pinned view's scrollbar: one glyph per transcript row, in the last
//! column. The thumb and the track differ by shape, not only by colour.

/// Marks the part of the transcript the view is showing.
pub const SCROLLBAR_THUMB: char = '┃';
/// Marks the rest of the transcript.
pub const SCROLLBAR_TRACK: char = '░';

/// The glyphs for a view `h` rows tall over a transcript of `total` rows,
/// with the view starting at row `top`. Empty when everything fits, so
/// there is nothing to draw.
///
/// With `m = total - h`: the thumb is `l = max(1, h * h / total)` rows long
/// and starts at `s = top * (h - l) / m` (0 when `m` is 0).
pub fn scrollbar_glyphs(h: i64, total: i64, top: i64) -> Vec<char> {
    if h <= 0 || total <= h {
        return Vec::new();
    }
    let max_top = total - h;
    let thumb = (h * h / total).max(1);
    let start = if max_top == 0 {
        0
    } else {
        top.clamp(0, max_top) * (h - thumb) / max_top
    };
    (0..h)
        .map(|row| {
            if row >= start && row < start + thumb {
                SCROLLBAR_THUMB
            } else {
                SCROLLBAR_TRACK
            }
        })
        .collect()
}
