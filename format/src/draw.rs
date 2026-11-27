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

//! Drawing an expanded format with its `#[]` styles, format-draw.c of
//! tmux 3.8.
//!
//! [`draw`] walks the expanded text once and puts each character into
//! one of eight sections: left, centre, right and absolute centre (from
//! `align=`), the list (`list=on`), its left and right markers
//! (`list=left-marker`, `list=right-marker`) and what follows the list.
//! Each section is a line of cells as screen_write_cell would fill a one
//! line screen, so wide characters take padding cells and zero width
//! characters join the cell before ([`tmux_utf8::combined`]). Then the
//! sections are trimmed to the available width and placed as C does:
//! the centre goes first, the list keeps its focus visible and shows its
//! markers when trimmed, and the absolute centre is drawn over the rest.
//!
//! The result ([`Drawn`]) is what to write, not a screen: a fill colour,
//! runs of cells at columns from the cursor (in order, later ones
//! overwrite earlier ones) and the ranges (`range=`) at their columns.
//! The adapter in tmux-rs writes it with screen_write.
//!
//! Styles are parsed by the caller ([`DrawHost::parse_style`], style.c's
//! style_parse) into [`Style`], the part of C's struct style that
//! drawing reads. The parser stays with the rest of style.c, which
//! options and every other style user share.
//!
//! Some of C's quirks are kept on purpose, as they decide what is drawn:
//! a range still open at the end is dropped, a range ends at the column
//! of the section current when it ends, ranges of a section that is not
//! drawn keep the columns they had in it, what follows a list is lost
//! when the list is trimmed away entirely, and offsets that go below
//! zero wrap (the adapter's cursor move clamps them, as in C).

use tmux_grid::{GRID_PADDING_CELL, GridCell, GridFlag};
use tmux_utf8::Utf8Data;
use tmux_utf8::combined::{Combine, utf8_combine};

use crate::parse::skip;
use crate::text::{Char, printable, read_char};

/// Where a style aligns what follows (`align=`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// No alignment given, which is left.
    #[default]
    Default,
    /// `align=left`.
    Left,
    /// `align=centre`.
    Centre,
    /// `align=right`.
    Right,
    /// `align=absolute-centre`.
    AbsoluteCentre,
}

impl Align {
    /// The index in C's map of alignments to sections.
    fn index(self) -> usize {
        match self {
            Align::Default => 0,
            Align::Left => 1,
            Align::Centre => 2,
            Align::Right => 3,
            Align::AbsoluteCentre => 4,
        }
    }
}

/// The list state of a style (`list=`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum List {
    /// Not in a list (`nolist`).
    #[default]
    Off,
    /// `list=on`.
    On,
    /// `list=focus`, the item to keep visible.
    Focus,
    /// `list=left-marker`, shown when the list is trimmed on the left.
    LeftMarker,
    /// `list=right-marker`, shown when the list is trimmed on the right.
    RightMarker,
}

/// The kind of a range (`range=`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RangeType {
    /// No range.
    #[default]
    None,
    /// `range=left`, the left of the status line.
    Left,
    /// `range=right`, the right of the status line.
    Right,
    /// `range=pane|%N`.
    Pane,
    /// `range=window|N`.
    Window,
    /// `range=session|$N`.
    Session,
    /// `range=user|text`.
    User,
    /// `range=control|N`, N from 0 to 9 (3.8).
    Control,
}

/// How a style changes the default colours (`push-default` and so on).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DefaultType {
    /// No change.
    #[default]
    Base,
    /// `push-default`: the style before this one becomes the default.
    Push,
    /// `pop-default`: back to the default drawing started with.
    Pop,
    /// `set-default`: the style before this one becomes the default and
    /// what `pop-default` goes back to (3.8).
    Set,
}

/// What drawing reads of a style (C's struct style).
#[derive(Clone, Copy)]
pub struct Style {
    /// The colours and attributes of the cells drawn with it. Its link
    /// is the hyperlink of the target screen, set while drawing.
    pub cell: GridCell,
    /// Whether `#[]` is drawn as text (`ignore`).
    pub ignore: bool,
    /// The fill colour (`fill=`), 8 for none.
    pub fill: i32,
    /// The alignment.
    pub align: Align,
    /// The list state.
    pub list: List,
    /// The kind of range.
    pub range_type: RangeType,
    /// The number of a pane, window, session or control range.
    pub range_argument: u32,
    /// The text of a user range, NUL padded, at most 15 bytes.
    pub range_string: [u8; 16],
    /// How the style changes the default.
    pub default_type: DefaultType,
    /// The style's hyperlink (`link=`), a number the parser hands out, 0
    /// for none.
    pub link: u32,
}

impl Style {
    /// style_set: the default style with the cell `base`.
    pub fn new(base: &GridCell) -> Self {
        Self {
            cell: *base,
            ignore: false,
            fill: 8,
            align: Align::Default,
            list: List::Off,
            range_type: RangeType::None,
            range_argument: 0,
            range_string: [0; 16],
            default_type: DefaultType::Base,
            link: 0,
        }
    }
}

/// What drawing needs from the caller.
pub trait DrawHost {
    /// style_parse: applies the style text `text` to `style`, with `base`
    /// as what `default` resets the colours and attributes to. Returns
    /// false when `text` is not a valid style, then `style` must be as it
    /// was.
    fn parse_style(&mut self, style: &mut Style, base: &GridCell, text: &[u8]) -> bool;

    /// The hyperlink the cells of the target screen carry for the style
    /// link `link` (never 0), 0 when the screen has no hyperlinks.
    fn link(&mut self, link: u32) -> u32 {
        let _ = link;
        0
    }

    /// Whether to build the lines C logs. Asked once per draw.
    fn logging(&self) -> bool {
        false
    }

    /// One line of the debug log, only called when
    /// [`DrawHost::logging`] said yes.
    fn log(&mut self, message: &str) {
        let _ = message;
    }
}

/// A range and the columns it covers, from the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    /// The kind.
    pub kind: RangeType,
    /// The number of a pane, window, session or control range.
    pub argument: u32,
    /// The text of a user range, NUL padded.
    pub string: [u8; 16],
    /// The first column.
    pub start: u32,
    /// The column after the last.
    pub end: u32,
}

/// Cells to write from a column. Padding cells follow wide characters.
#[derive(Clone)]
pub struct Put {
    /// The column, from the cursor. It may have wrapped below zero as in
    /// C, the cursor move that places it clamps it to the screen.
    pub x: u32,
    /// The cells, one per column.
    pub cells: Vec<GridCell>,
}

/// What [`draw`] found to draw.
#[derive(Clone, Default)]
pub struct Drawn {
    /// The fill colour (`fill=`): when set, the available width is first
    /// filled with spaces of the default cell with this background,
    /// written from the cursor.
    pub fill: Option<i32>,
    /// The runs of cells to write, in order.
    pub puts: Vec<Put>,
    /// The ranges, when asked for.
    pub ranges: Vec<Range>,
}

/// The sections of the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Left,
    Centre,
    Right,
    AbsoluteCentre,
    List,
    ListLeft,
    ListRight,
    After,
}

impl Section {
    const ALL: [Section; 8] = [
        Section::Left,
        Section::Centre,
        Section::Right,
        Section::AbsoluteCentre,
        Section::List,
        Section::ListLeft,
        Section::ListRight,
        Section::After,
    ];

    fn name(self) -> &'static str {
        match self {
            Section::Left => "LEFT",
            Section::Centre => "CENTRE",
            Section::Right => "RIGHT",
            Section::AbsoluteCentre => "ABSOLUTE_CENTRE",
            Section::List => "LIST",
            Section::ListLeft => "LIST_LEFT",
            Section::ListRight => "LIST_RIGHT",
            Section::After => "AFTER",
        }
    }
}

/// One section: the cells of a one line screen, the cursor after the
/// last.
#[derive(Default)]
struct Line {
    cells: Vec<GridCell>,
}

impl Line {
    /// The cursor, C's `s->cx`.
    fn cx(&self) -> u32 {
        self.cells.len() as u32
    }

    /// Writes a cell at the cursor as screen_write_cell does on a line
    /// wide enough for everything: a zero width character joins the cell
    /// before or is dropped, a wide one is followed by padding.
    fn write(&mut self, gc: &GridCell) {
        let cx = self.cells.len();
        let mut last = None;
        if cx != 0 {
            let mut n = 1;
            if cx != 1 && self.cells[cx - 1].flags.intersects(GridFlag::PADDING) {
                n = 2;
            }
            let cell = &self.cells[cx - n];
            if usize::from(cell.data.width) == n && !cell.flags.intersects(GridFlag::PADDING) {
                last = Some(cx - n);
            }
        }
        match utf8_combine(last.map(|i| &self.cells[i].data), &gc.data) {
            Combine::Discard => {}
            Combine::Combined { data, widen } => {
                let Some(i) = last else {
                    unreachable!("combined without a cell");
                };
                self.cells[i].data = data;
                if widen {
                    self.cells.push(GRID_PADDING_CELL);
                }
            }
            Combine::Separate => {
                self.cells.push(*gc);
                for _ in 1..gc.data.width {
                    self.cells.push(GRID_PADDING_CELL);
                }
            }
        }
    }

    /// Writes `n` copies of the ASCII character `ch` (format_draw_many).
    fn write_many(&mut self, style: &GridCell, ch: u8, n: usize) {
        let mut gc = *style;
        gc.data = Utf8Data::single(ch);
        for _ in 0..n {
            self.write(&gc);
        }
    }

    /// The cells screen_write_fast_copy copies from column `start`: up
    /// to `width` columns, stopping before a character that does not fit.
    fn copy(&self, start: u32, width: u32) -> Vec<GridCell> {
        let end = start + width;
        debug_assert!(end <= self.cx(), "copy past the cursor");
        let mut out = Vec::with_capacity(width as usize);
        for xx in start..end {
            let Some(gc) = self.cells.get(xx as usize) else {
                break;
            };
            if xx + u32::from(gc.data.width) > end {
                break;
            }
            out.push(*gc);
        }
        out
    }
}

/// A range while drawing (C's struct format_range): its section and the
/// columns in it, or on the output once the section is placed.
#[derive(Clone, Copy)]
struct SectionRange {
    section: Section,
    start: u32,
    end: u32,
    kind: RangeType,
    argument: u32,
    string: [u8; 16],
}

impl SectionRange {
    /// format_is_type: whether `sy` continues this range.
    fn is_type(&self, sy: &Style) -> bool {
        if self.kind != sy.range_type {
            return false;
        }
        match self.kind {
            RangeType::None | RangeType::Left | RangeType::Right | RangeType::Control => true,
            RangeType::Pane | RangeType::Window | RangeType::Session => {
                self.argument == sy.range_argument
            }
            RangeType::User => until_nul(&self.string) == until_nul(&sy.range_string),
        }
    }
}

/// The bytes of a NUL padded field up to its first NUL.
fn until_nul(s: &[u8]) -> &[u8] {
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}

/// The output while the sections are placed.
struct Output<'a> {
    lines: &'a [Line; 8],
    puts: Vec<Put>,
    ranges: Vec<SectionRange>,
}

impl Output<'_> {
    fn line(&self, section: Section) -> &Line {
        &self.lines[section as usize]
    }

    fn cx(&self, section: Section) -> u32 {
        self.line(section).cx()
    }

    /// Copies cells of a section to a column without touching ranges.
    /// An empty copy is kept too: C moves the cursor for it, and a later
    /// column that wraps to -1 leaves the cursor where that put it.
    fn copy(&mut self, section: Section, x: u32, start: u32, width: u32) {
        let cells = if width == 0 {
            Vec::new()
        } else {
            self.line(section).copy(start, width)
        };
        self.puts.push(Put { x, cells });
    }

    /// format_draw_put: draws `width` columns of a section from `start`
    /// at `offset`, and moves its ranges there (format_update_ranges).
    fn put(&mut self, section: Section, offset: u32, start: u32, width: u32) {
        self.copy(section, offset, start, width);
        self.ranges.retain_mut(|fr| {
            if fr.section != section {
                return true;
            }
            if fr.end <= start || fr.start >= start + width {
                return false;
            }
            fr.start = fr.start.max(start);
            fr.end = fr.end.min(start + width);
            if fr.start == fr.end {
                return false;
            }
            fr.start = (fr.start - start).wrapping_add(offset);
            fr.end = (fr.end - start).wrapping_add(offset);
            true
        });
    }

    /// format_draw_put_list: draws the list in `width` columns at
    /// `offset`. When it does not fit, the focus stays visible and the
    /// markers show on the sides that are cut.
    fn put_list(&mut self, mut offset: u32, mut width: u32, focus_start: i32, focus_end: i32) {
        let list = self.cx(Section::List);

        // If there is enough space for the list, draw it entirely.
        if width >= list {
            self.put(Section::List, offset, 0, width);
            return;
        }

        // The list needs to be trimmed. Try to keep the focus visible.
        let focus_centre = (focus_start + (focus_end - focus_start) / 2) as u32;
        let mut start = focus_centre.saturating_sub(width / 2);
        if start + width > list {
            start = list - width;
        }

        // Draw <> markers at either side if needed.
        let left = self.cx(Section::ListLeft);
        if start != 0 && width > left {
            self.copy(Section::ListLeft, offset, 0, left);
            offset = offset.wrapping_add(left);
            start += left;
            width -= left;
        }
        let right = self.cx(Section::ListRight);
        if start + width < list && width > right {
            self.copy(
                Section::ListRight,
                offset.wrapping_add(width - right),
                0,
                right,
            );
            width -= right;
        }

        // Draw the list itself.
        self.put(Section::List, offset, start, width);
    }

    /// format_draw_none: no list.
    fn none(&mut self, available: u32) {
        let mut width_left = self.cx(Section::Left);
        let mut width_centre = self.cx(Section::Centre);
        let mut width_right = self.cx(Section::Right);
        let mut width_abs_centre = self.cx(Section::AbsoluteCentre);

        // Keep as much of the left and right as possible at the expense
        // of the centre.
        while width_left + width_centre + width_right > available {
            if width_centre > 0 {
                width_centre -= 1;
            } else if width_right > 0 {
                width_right -= 1;
            } else {
                width_left -= 1;
            }
        }

        // Left, then right at the end.
        self.put(Section::Left, 0, 0, width_left);
        self.put(
            Section::Right,
            available - width_right,
            self.cx(Section::Right) - width_right,
            width_right,
        );

        // The centre halfway between the left and the right.
        self.put(
            Section::Centre,
            (width_left + ((available - width_right) - width_left) / 2)
                .wrapping_sub(width_centre / 2),
            self.cx(Section::Centre) / 2 - width_centre / 2,
            width_centre,
        );

        // The absolute centre in the centre of all the space.
        width_abs_centre = width_abs_centre.min(available);
        self.put(
            Section::AbsoluteCentre,
            (available - width_abs_centre) / 2,
            0,
            width_abs_centre,
        );
    }

    /// format_draw_left: the list is part of the left.
    fn left(&mut self, available: u32, focus_start: i32, focus_end: i32) {
        let mut width_left = self.cx(Section::Left);
        let mut width_centre = self.cx(Section::Centre);
        let mut width_right = self.cx(Section::Right);
        let mut width_abs_centre = self.cx(Section::AbsoluteCentre);
        let mut width_list = self.cx(Section::List);
        let mut width_after = self.cx(Section::After);

        // Trim first the centre, then the list, then the right, then
        // after the list, then the left.
        while width_left + width_centre + width_right + width_list + width_after > available {
            if width_centre > 0 {
                width_centre -= 1;
            } else if width_list > 0 {
                width_list -= 1;
            } else if width_right > 0 {
                width_right -= 1;
            } else if width_after > 0 {
                width_after -= 1;
            } else {
                width_left -= 1;
            }
        }

        // With no list left, C copies what follows it past the cursor of
        // the left, where nothing reads it, and draws without a list.
        if width_list == 0 {
            self.none(available);
            return;
        }

        self.put(Section::Left, 0, 0, width_left);
        self.put(
            Section::Right,
            available - width_right,
            self.cx(Section::Right) - width_right,
            width_right,
        );
        self.put(Section::After, width_left + width_list, 0, width_after);

        // The centre halfway between after the list and the right.
        let before = width_left + width_list + width_after;
        self.put(
            Section::Centre,
            (before + ((available - width_right) - before) / 2).wrapping_sub(width_centre / 2),
            self.cx(Section::Centre) / 2 - width_centre / 2,
            width_centre,
        );

        // Without a focus, keep the left of the list in view.
        let (focus_start, focus_end) = if focus_start == -1 || focus_end == -1 {
            (0, 0)
        } else {
            (focus_start, focus_end)
        };
        self.put_list(width_left, width_list, focus_start, focus_end);

        width_abs_centre = width_abs_centre.min(available);
        self.put(
            Section::AbsoluteCentre,
            (available - width_abs_centre) / 2,
            0,
            width_abs_centre,
        );
    }

    /// format_draw_centre: the list is part of the centre.
    fn centre(&mut self, available: u32, focus_start: i32, focus_end: i32) {
        let mut width_left = self.cx(Section::Left);
        let mut width_centre = self.cx(Section::Centre);
        let mut width_right = self.cx(Section::Right);
        let mut width_abs_centre = self.cx(Section::AbsoluteCentre);
        let mut width_list = self.cx(Section::List);
        let mut width_after = self.cx(Section::After);

        // Trim first the list, then after the list, then the centre, then
        // the right, then the left.
        while width_left + width_centre + width_right + width_list + width_after > available {
            if width_list > 0 {
                width_list -= 1;
            } else if width_after > 0 {
                width_after -= 1;
            } else if width_centre > 0 {
                width_centre -= 1;
            } else if width_right > 0 {
                width_right -= 1;
            } else {
                width_left -= 1;
            }
        }

        if width_list == 0 {
            self.none(available);
            return;
        }

        self.put(Section::Left, 0, 0, width_left);
        self.put(
            Section::Right,
            available - width_right,
            self.cx(Section::Right) - width_right,
            width_right,
        );

        // The centre, the list and after it are placed around the middle
        // of the space between the left and the right.
        let middle = width_left + ((available - width_right) - width_left) / 2;
        self.put(
            Section::Centre,
            middle
                .wrapping_sub(width_list / 2)
                .wrapping_sub(width_centre),
            0,
            width_centre,
        );
        self.put(
            Section::After,
            middle.wrapping_sub(width_list / 2).wrapping_add(width_list),
            0,
            width_after,
        );

        // Without a focus, keep the middle of the list in view.
        let (focus_start, focus_end) = if focus_start == -1 || focus_end == -1 {
            let half = (self.cx(Section::List) / 2) as i32;
            (half, half)
        } else {
            (focus_start, focus_end)
        };
        self.put_list(
            middle.wrapping_sub(width_list / 2),
            width_list,
            focus_start,
            focus_end,
        );

        width_abs_centre = width_abs_centre.min(available);
        self.put(
            Section::AbsoluteCentre,
            (available - width_abs_centre) / 2,
            0,
            width_abs_centre,
        );
    }

    /// format_draw_right: the list is part of the right.
    fn right(&mut self, available: u32, focus_start: i32, focus_end: i32) {
        let mut width_left = self.cx(Section::Left);
        let mut width_centre = self.cx(Section::Centre);
        let mut width_right = self.cx(Section::Right);
        let mut width_abs_centre = self.cx(Section::AbsoluteCentre);
        let mut width_list = self.cx(Section::List);
        let mut width_after = self.cx(Section::After);

        // Trim first the centre, then the list, then the right, then
        // after the list, then the left.
        while width_left + width_centre + width_right + width_list + width_after > available {
            if width_centre > 0 {
                width_centre -= 1;
            } else if width_list > 0 {
                width_list -= 1;
            } else if width_right > 0 {
                width_right -= 1;
            } else if width_after > 0 {
                width_after -= 1;
            } else {
                width_left -= 1;
            }
        }

        if width_list == 0 {
            self.none(available);
            return;
        }

        self.put(Section::Left, 0, 0, width_left);
        self.put(
            Section::After,
            available - width_after,
            self.cx(Section::After) - width_after,
            width_after,
        );
        let before_list = available - width_right - width_list - width_after;
        self.put(Section::Right, before_list, 0, width_right);

        // The centre halfway between the left and the right.
        self.put(
            Section::Centre,
            (width_left + (before_list - width_left) / 2).wrapping_sub(width_centre / 2),
            self.cx(Section::Centre) / 2 - width_centre / 2,
            width_centre,
        );

        // Without a focus, keep the left of the list in view.
        let (focus_start, focus_end) = if focus_start == -1 || focus_end == -1 {
            (0, 0)
        } else {
            (focus_start, focus_end)
        };
        self.put_list(
            available - width_list - width_after,
            width_list,
            focus_start,
            focus_end,
        );

        width_abs_centre = width_abs_centre.min(available);
        self.put(
            Section::AbsoluteCentre,
            (available - width_abs_centre) / 2,
            0,
            width_abs_centre,
        );
    }

    /// format_draw_absolute_centre: the list is in the centre of all the
    /// space, with the absolute centre before it and after it after.
    fn absolute_centre(&mut self, available: u32, focus_start: i32, focus_end: i32) {
        let mut width_left = self.cx(Section::Left);
        let mut width_centre = self.cx(Section::Centre);
        let mut width_right = self.cx(Section::Right);
        let mut width_abs_centre = self.cx(Section::AbsoluteCentre);
        let mut width_list = self.cx(Section::List);
        let mut width_after = self.cx(Section::After);

        // Trim first the centre, then the right, then the left.
        while width_left + width_centre + width_right > available {
            if width_centre > 0 {
                width_centre -= 1;
            } else if width_right > 0 {
                width_right -= 1;
            } else {
                width_left -= 1;
            }
        }

        // The list, after it and the absolute centre are drawn over the
        // rest, so they are trimmed on their own: first the list, then
        // after it, then the absolute centre.
        while width_list + width_after + width_abs_centre > available {
            if width_list > 0 {
                width_list -= 1;
            } else if width_after > 0 {
                width_after -= 1;
            } else {
                width_abs_centre -= 1;
            }
        }

        self.put(Section::Left, 0, 0, width_left);
        self.put(
            Section::Right,
            available - width_right,
            self.cx(Section::Right) - width_right,
            width_right,
        );

        // The centre stays at the relative centre.
        let middle = width_left + ((available - width_right) - width_left) / 2;
        self.put(
            Section::Centre,
            middle.wrapping_sub(width_centre),
            0,
            width_centre,
        );

        // Without a focus, keep the middle of the list in view.
        let (focus_start, focus_end) = if focus_start == -1 || focus_end == -1 {
            let half = (self.cx(Section::List) / 2) as i32;
            (half, half)
        } else {
            (focus_start, focus_end)
        };

        // The absolute centre and the list are centred together, then
        // after the list follows.
        let mut offset = (available - width_list - width_abs_centre) / 2;
        self.put(Section::AbsoluteCentre, offset, 0, width_abs_centre);
        offset += width_abs_centre;
        self.put_list(offset, width_list, focus_start, focus_end);
        offset += width_list;
        self.put(Section::After, offset, 0, width_after);
    }
}

/// The byte at `i`, or 0 past the end, as C reads its terminator.
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// format_draw: lays out `expanded` in `available` columns with the cell
/// `base` as the default style. With `ranges` set the ranges are worked
/// out, and with `default_colours` set every style keeps the colours of
/// the default. A style without its closing `]` draws nothing.
pub fn draw<H: DrawHost + ?Sized>(
    host: &mut H,
    base: &GridCell,
    available: u32,
    expanded: &[u8],
    ranges: bool,
    default_colours: bool,
) -> Drawn {
    // C stops at the first NUL.
    let s = &expanded[..expanded
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(expanded.len())];
    let logging = host.logging();
    if logging {
        host.log(&format!("format_draw: {}", String::from_utf8_lossy(s)));
    }

    let mut base_default = *base;
    let mut current_default = *base;
    let mut sy = Style::new(&current_default);

    let mut lines: [Line; 8] = Default::default();
    let mut current = Section::Left;
    let mut last = Section::Left;
    let mut map = [
        Section::Left,
        Section::Left,
        Section::Centre,
        Section::Right,
        Section::AbsoluteCentre,
    ];
    let mut focus_start: i32 = -1;
    let mut focus_end: i32 = -1;
    let mut list_state = -1;
    let mut fill: i32 = -1;
    let mut list_align = Align::Default;
    let mut open: Option<SectionRange> = None;
    let mut frs: Vec<SectionRange> = Vec::new();

    // Walk the string and add to the sections, parsing styles as we go.
    let mut i = 0;
    while i < s.len() {
        // Sequences of #.
        if s[i] == b'#' && at(s, i + 1) != b'[' && at(s, i + 1) != 0 {
            let mut n = 1;
            while at(s, i + n) == b'#' {
                n += 1;
            }
            let even = n % 2 == 0;
            if at(s, i + n) != b'[' {
                i += n;
                lines[current as usize].write_many(&sy.cell, b'#', n.div_ceil(2));
                continue;
            }
            i += if even { n + 1 } else { n - 1 };
            if sy.ignore {
                continue;
            }
            lines[current as usize].write_many(&sy.cell, b'#', n / 2);
            if even {
                lines[current as usize].write_many(&sy.cell, b'[', 1);
            }
            continue;
        }

        // Not a style: a UTF-8 character, printable ASCII or nothing.
        if s[i] != b'#' || at(s, i + 1) != b'[' || sy.ignore {
            let mut gc = sy.cell;
            match read_char(s, i) {
                Some(Char::Done { size, width }) => {
                    let mut ud = Utf8Data::empty();
                    ud.data[..size].copy_from_slice(&s[i..i + size]);
                    ud.have = size as u8;
                    ud.size = size as u8;
                    ud.width = width as u8;
                    gc.data = ud;
                    i += size;
                }
                // C goes back to the lead byte of an unfinished or
                // invalid character, which is not printable.
                _ => {
                    if !printable(s[i]) {
                        i += 1;
                        continue;
                    }
                    gc.data = Utf8Data::single(s[i]);
                    i += 1;
                }
            }
            lines[current as usize].write(&gc);
            continue;
        }

        // A style. Work out where it ends and parse it.
        let Some(end) = skip(s, i + 2, b"]") else {
            if logging {
                host.log(&format!(
                    "format_draw: no terminating ] at '{}'",
                    String::from_utf8_lossy(&s[i + 2..])
                ));
            }
            return Drawn::default();
        };
        let text = &s[i + 2..end];
        i = end + 1;
        let saved = sy;
        if !host.parse_style(&mut sy, &current_default, text) {
            sy = saved;
            if logging {
                host.log(&format!(
                    "format_draw: invalid style '{}'",
                    String::from_utf8_lossy(text)
                ));
            }
            continue;
        }
        if default_colours {
            sy.cell.bg = base_default.bg;
            sy.cell.fg = base_default.fg;
        }

        // The hyperlink of the style, as the target screen numbers it.
        sy.cell.link = if sy.link != 0 { host.link(sy.link) } else { 0 };

        // A fill colour is kept for later.
        if sy.fill != 8 {
            fill = sy.fill;
        }

        // Push, pop or set the default.
        match sy.default_type {
            DefaultType::Base => {}
            DefaultType::Push => current_default = saved.cell,
            DefaultType::Pop => current_default = base_default,
            DefaultType::Set => {
                base_default = saved.cell;
                current_default = saved.cell;
            }
        }
        sy.default_type = DefaultType::Base;

        // The list state.
        let list_cx = lines[Section::List as usize].cx() as i32;
        match sy.list {
            List::On => {
                // Entering the list, leaving a marker or leaving the
                // focus.
                if list_state != 0 {
                    open = None;
                    list_state = 0;
                    list_align = sy.align;
                }
                if focus_start != -1 && focus_end == -1 {
                    focus_end = list_cx;
                }
                current = Section::List;
            }
            List::Focus => {
                // Entering the focus, only inside the list.
                if list_state == 0 && focus_start == -1 {
                    focus_start = list_cx;
                }
            }
            List::Off => {
                // Leaving or outside the list.
                if list_state == 0 {
                    open = None;
                    if focus_start != -1 && focus_end == -1 {
                        focus_end = list_cx;
                    }
                    map[list_align.index()] = Section::After;
                    if list_align == Align::Left {
                        map[Align::Default.index()] = Section::After;
                    }
                    list_state = 1;
                }
                current = map[sy.align.index()];
            }
            List::LeftMarker | List::RightMarker => {
                // Entering a marker, only inside the list and only the
                // first time.
                let marker = if sy.list == List::LeftMarker {
                    Section::ListLeft
                } else {
                    Section::ListRight
                };
                if list_state == 0 && lines[marker as usize].cx() == 0 {
                    open = None;
                    if focus_start != -1 && focus_end == -1 {
                        focus_start = -1;
                        focus_end = -1;
                    }
                    current = marker;
                }
            }
        }
        if current != last {
            if logging {
                host.log(&format!(
                    "format_draw: change {} -> {}",
                    last.name(),
                    current.name()
                ));
            }
            last = current;
        }

        // When the range changes, end the open one and start a new one.
        if ranges {
            let cx = lines[current as usize].cx();
            if let Some(fr) = open
                && !fr.is_type(&sy)
            {
                if cx != fr.start {
                    frs.push(SectionRange { end: cx, ..fr });
                }
                open = None;
            }
            if open.is_none() && sy.range_type != RangeType::None {
                open = Some(SectionRange {
                    section: current,
                    start: cx,
                    end: 0,
                    kind: sy.range_type,
                    argument: sy.range_argument,
                    string: sy.range_string,
                });
            }
        }
    }
    // A range still open at the end is dropped.

    if logging {
        for section in Section::ALL {
            host.log(&format!(
                "format_draw: width {} is {}",
                section.name(),
                lines[section as usize].cx()
            ));
        }
        if focus_start != -1 && focus_end != -1 {
            host.log(&format!("format_draw: focus {focus_start}-{focus_end}"));
        }
        for fr in &frs {
            host.log(&format!(
                "format_draw: range {}|{} is {} {}-{}",
                fr.kind as u32,
                fr.argument,
                fr.section.name(),
                fr.start,
                fr.end
            ));
        }
    }

    // Place the sections. How depends on where the list is.
    let mut out = Output {
        lines: &lines,
        puts: Vec::new(),
        ranges: frs,
    };
    match list_align {
        Align::Default => out.none(available),
        Align::Left => out.left(available, focus_start, focus_end),
        Align::Centre => out.centre(available, focus_start, focus_end),
        Align::Right => out.right(available, focus_start, focus_end),
        Align::AbsoluteCentre => out.absolute_centre(available, focus_start, focus_end),
    }

    let ranges: Vec<Range> = out
        .ranges
        .iter()
        .map(|fr| Range {
            kind: fr.kind,
            argument: fr.argument,
            string: fr.string,
            start: fr.start,
            end: fr.end,
        })
        .collect();
    if logging {
        for r in &ranges {
            let what = match r.kind {
                RangeType::None => continue,
                RangeType::Left => "left".to_string(),
                RangeType::Right => "right".to_string(),
                RangeType::Pane => format!("pane|%{}", r.argument),
                RangeType::Window => format!("window|{}", r.argument),
                RangeType::Session => format!("session|${}", r.argument),
                RangeType::User => format!("user|{}", r.argument),
                RangeType::Control => format!("control|{}", r.argument),
            };
            host.log(&format!(
                "format_draw: range {what} at {}-{}",
                r.start, r.end
            ));
        }
    }

    Drawn {
        fill: (fill != -1).then_some(fill),
        puts: out.puts,
        ranges,
    }
}

#[cfg(test)]
mod tests;
