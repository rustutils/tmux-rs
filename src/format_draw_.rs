// Copyright (c) 2019 Nicholas Marriott <nicholas.marriott@gmail.com>
//
// Permission to use, copy, modify, and distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
// IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
// OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

//! Drawing an expanded format to a screen (format-draw.c).
//!
//! The layout is [`tmux_format::draw`]. This adapter parses styles for
//! it with style.c's parser, gives it the hyperlinks of the target
//! screen, and writes what it returns with `screen_write`: the fill, the
//! runs of cells at their columns, and the ranges.

use tmux_format::draw::{self, Align, DefaultType, DrawHost, List, RangeType, Style};

use crate::*;

/// The style of the crate as C's struct style.
fn style_to_c(sy: &Style) -> style {
    style {
        gc: sy.cell,
        ignore: i32::from(sy.ignore),
        fill: sy.fill,
        align: match sy.align {
            Align::Default => style_align::STYLE_ALIGN_DEFAULT,
            Align::Left => style_align::STYLE_ALIGN_LEFT,
            Align::Centre => style_align::STYLE_ALIGN_CENTRE,
            Align::Right => style_align::STYLE_ALIGN_RIGHT,
            Align::AbsoluteCentre => style_align::STYLE_ALIGN_ABSOLUTE_CENTRE,
        },
        list: match sy.list {
            List::Off => style_list::STYLE_LIST_OFF,
            List::On => style_list::STYLE_LIST_ON,
            List::Focus => style_list::STYLE_LIST_FOCUS,
            List::LeftMarker => style_list::STYLE_LIST_LEFT_MARKER,
            List::RightMarker => style_list::STYLE_LIST_RIGHT_MARKER,
        },
        range_type: range_type_to_c(sy.range_type),
        range_argument: sy.range_argument,
        range_string: sy.range_string,
        default_type: match sy.default_type {
            DefaultType::Base => style_default_type::STYLE_DEFAULT_BASE,
            DefaultType::Push => style_default_type::STYLE_DEFAULT_PUSH,
            DefaultType::Pop => style_default_type::STYLE_DEFAULT_POP,
            DefaultType::Set => style_default_type::STYLE_DEFAULT_SET,
        },
        link: sy.link,
    }
}

/// C's struct style as the style of the crate.
fn style_from_c(sy: &style) -> Style {
    Style {
        cell: sy.gc,
        ignore: sy.ignore != 0,
        fill: sy.fill,
        align: match sy.align {
            style_align::STYLE_ALIGN_DEFAULT => Align::Default,
            style_align::STYLE_ALIGN_LEFT => Align::Left,
            style_align::STYLE_ALIGN_CENTRE => Align::Centre,
            style_align::STYLE_ALIGN_RIGHT => Align::Right,
            style_align::STYLE_ALIGN_ABSOLUTE_CENTRE => Align::AbsoluteCentre,
        },
        list: match sy.list {
            style_list::STYLE_LIST_OFF => List::Off,
            style_list::STYLE_LIST_ON => List::On,
            style_list::STYLE_LIST_FOCUS => List::Focus,
            style_list::STYLE_LIST_LEFT_MARKER => List::LeftMarker,
            style_list::STYLE_LIST_RIGHT_MARKER => List::RightMarker,
        },
        range_type: match sy.range_type {
            style_range_type::STYLE_RANGE_NONE => RangeType::None,
            style_range_type::STYLE_RANGE_LEFT => RangeType::Left,
            style_range_type::STYLE_RANGE_RIGHT => RangeType::Right,
            style_range_type::STYLE_RANGE_PANE => RangeType::Pane,
            style_range_type::STYLE_RANGE_WINDOW => RangeType::Window,
            style_range_type::STYLE_RANGE_SESSION => RangeType::Session,
            style_range_type::STYLE_RANGE_USER => RangeType::User,
        },
        range_argument: sy.range_argument,
        range_string: sy.range_string,
        default_type: match sy.default_type {
            style_default_type::STYLE_DEFAULT_BASE => DefaultType::Base,
            style_default_type::STYLE_DEFAULT_PUSH => DefaultType::Push,
            style_default_type::STYLE_DEFAULT_POP => DefaultType::Pop,
            style_default_type::STYLE_DEFAULT_SET => DefaultType::Set,
        },
        link: sy.link,
    }
}

/// A range kind of the crate as C's. style.c does not parse control
/// ranges yet (they wait for `STYLE_RANGE_CONTROL` in src/lib.rs), so the
/// crate never returns one here.
fn range_type_to_c(kind: RangeType) -> style_range_type {
    match kind {
        RangeType::None => style_range_type::STYLE_RANGE_NONE,
        RangeType::Left => style_range_type::STYLE_RANGE_LEFT,
        RangeType::Right => style_range_type::STYLE_RANGE_RIGHT,
        RangeType::Pane => style_range_type::STYLE_RANGE_PANE,
        RangeType::Window => style_range_type::STYLE_RANGE_WINDOW,
        RangeType::Session => style_range_type::STYLE_RANGE_SESSION,
        RangeType::User => style_range_type::STYLE_RANGE_USER,
        RangeType::Control => {
            debug_assert!(false, "control range without a parser for it");
            style_range_type::STYLE_RANGE_NONE
        }
    }
}

/// What the layout asks of tmux-rs: style.c's parser, the hyperlinks of
/// the screen drawn to and the server log.
struct ScreenHost {
    /// The hyperlinks of the target screen, if it has them.
    hyperlinks: Option<*mut hyperlinks>,
    /// Whether the server logs.
    logging: bool,
}

impl DrawHost for ScreenHost {
    fn parse_style(&mut self, sy: &mut Style, base: &GridCell, text: &[u8]) -> bool {
        // The layout cut the text at the first NUL, so it has none.
        let Ok(text) = CString::new(text) else {
            return false;
        };
        let mut c = style_to_c(sy);
        // SAFETY: the style and the base are references and the text a C
        // string, all valid through the call. style_parse reads the
        // colour table and the style links store, nothing it could free.
        if unsafe { style_parse(&raw mut c, base, text.as_ptr().cast()) } != 0 {
            return false;
        }
        if self.logging {
            log_debug!(
                "format_draw: style '{}' -> '{}'",
                text.to_string_lossy(),
                style_tostring(&c)
            );
        }
        *sy = style_from_c(&c);
        true
    }

    fn link(&mut self, link: u32) -> u32 {
        let Some(hl) = self.hyperlinks else {
            return 0;
        };
        let Some(uri) = crate::global::with_style_links(|links| links.get(link).map(<[u8]>::to_vec))
        else {
            return 0;
        };
        let Ok(uri) = CString::new(uri) else {
            return 0;
        };
        // SAFETY: hl is the hyperlinks set of the screen being drawn,
        // which lives while format_draw runs. The URI doubles as the
        // internal ID, so repeated links share one entry and the ID stays
        // stable across redraws.
        unsafe { hyperlinks_put(hl, uri.as_ptr().cast(), uri.as_ptr().cast()) }
    }

    fn logging(&self) -> bool {
        self.logging
    }

    fn log(&mut self, message: &str) {
        log_debug!("{message}");
    }
}

/// Draws the expanded format `expanded` at the cursor of `octx` in
/// `available` columns, with `base` as the default cell. Adds the ranges
/// to `srs` when it is not null. With `default_colours` every style keeps
/// the colours of the default. The cursor is left where it was.
///
/// # Safety
/// `octx` must have been started on a valid screen, `base` must be
/// valid, and `srs` null or valid.
pub unsafe fn format_draw(
    octx: *mut screen_write_ctx,
    base: *const GridCell,
    available: c_uint,
    expanded: &str,
    srs: *mut style_ranges,
    default_colours: c_int,
) {
    // SAFETY: the caller passes a started context with a valid screen,
    // a valid base cell and null or valid ranges.
    unsafe {
        let os = (*octx).s;
        let (ocx, ocy) = ((*os).cx, (*os).cy);
        let mut host = ScreenHost {
            hyperlinks: (*os).hyperlinks,
            logging: log_get_level() != 0,
        };
        let drawn = draw::draw(
            &mut host,
            &*base,
            available,
            expanded.as_bytes(),
            !srs.is_null(),
            default_colours != 0,
        );

        // Clear the available area.
        if let Some(fill) = drawn.fill {
            let mut gc = GRID_DEFAULT_CELL;
            gc.bg = fill;
            for _ in 0..available {
                screen_write_putc(octx, &gc, b' ');
            }
        }

        // Write each run where it goes, as screen_write_fast_copy writes
        // cells: into the grid, from the cursor, without moving it. The
        // column wraps as C's unsigned sum does and the move clamps it.
        for put in &drawn.puts {
            screen_write_cursormove(octx, ocx.wrapping_add(put.x) as i32, ocy as i32, 0);
            let (cx, cy) = ((*os).cx, (*os).cy);
            for (x, gc) in (cx..).zip(&put.cells) {
                (*os).grid.view_set_cell(x, cy, gc);
            }
        }

        if !srs.is_null() {
            (*srs).extend(drawn.ranges.iter().map(|r| style_range {
                type_: range_type_to_c(r.kind),
                argument: r.argument,
                string: r.string,
                start: r.start,
                end: r.end,
            }));
        }

        // Restore the original cursor position.
        screen_write_cursormove(octx, ocx as i32, ocy as i32, 0);
    }
}

// format_skip in src/format.rs lost its last caller with the layout
// moving to tmux-format. That file belongs to another part of run 20, so
// it is referenced here until it is removed there.
const _: () = {
    let _ = format_skip;
};

/// The display width of an expanded format, without its styles
/// (`format_width`, from the format crate).
pub fn format_width(expanded: &str) -> u32 {
    tmux_format::format_width(expanded.as_bytes())
}

/// Trims `expanded` to its last `limit` columns, keeping styles
/// (`format_trim_right`, from the format crate).
///
/// # Safety
/// `expanded` must be a C string.
pub unsafe fn format_trim_right(expanded: *const u8, limit: u32) -> *mut u8 {
    // SAFETY: the caller passes a C string.
    let bytes = unsafe { crate::format_env::format_bytes(expanded) };
    crate::format_env::format_to_c(&tmux_format::trim_right(bytes, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3.8's default status-format[0] expanded for a session "main" with
    /// three windows, the second current (display -p '#{T:status-format[0]}'
    /// on tmux 3.8).
    pub(super) const STATUS_38: &str = "#[align=left range=left default]#[push-default][main] #[pop-default]#[norange default]#[list=on align=left]#[list=left-marker]<#[list=right-marker]>#[list=on]#[range=window|0 default]#[push-default]0:tmux #[pop-default]#[norange default] #[range=window|1 list=focus underscore]#[push-default]1:tmux*#[pop-default]#[norange list=on default] #[range=window|2 default]#[push-default]2:editor- #[pop-default]#[norange default]#[nolist align=right range=right default]#[push-default]\"server\" 19:51 27-Nov-26#[pop-default]#[norange default]";

    /// Times drawing 3.8's default status line into a 120 column screen
    /// with ranges. Run with `cargo test --release --lib
    /// format_draw_::tests::draw_status_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing, run by hand"]
    fn draw_status_cost() {
        let n = 20_000u32;
        // SAFETY: the screen is initialised before use and freed after,
        // nothing else runs in this test.
        unsafe {
            let s = crate::screen_::tests::make_screen(120, 1);
            let mut ctx: screen_write_ctx = zeroed();
            let mut ranges: style_ranges = Vec::new();
            let start = std::time::Instant::now();
            for _ in 0..n {
                ranges.clear();
                screen_write_start(&raw mut ctx, s);
                screen_write_cursormove(&raw mut ctx, 0, 0, 0);
                format_draw(&raw mut ctx, &raw const GRID_DEFAULT_CELL, 120, STATUS_38, &raw mut ranges, 0);
                screen_write_stop(&raw mut ctx);
            }
            let each = start.elapsed() / n;
            eprintln!("format_draw of 3.8's status line: {each:?}, {} ranges", ranges.len());
            crate::screen_::tests::destroy_screen(s);
        }
    }
}
