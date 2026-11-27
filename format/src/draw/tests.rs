//! Tests of the layout. The style parser here knows the words the tests
//! use, with style.c's rules (styles add up, `default` goes back to the
//! base, an invalid word leaves the style as it was).

use tmux_grid::{GRID_DEFAULT_CELL, GridAttr, GridCell, GridFlag};

use super::*;

/// A host with a small style parser and links numbered from 100.
#[derive(Default)]
struct Host {
    links: Vec<u32>,
    log: Vec<String>,
    logging: bool,
}

fn colour(name: &str) -> Option<i32> {
    Some(match name {
        "default" => 8,
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        _ => return None,
    })
}

impl DrawHost for Host {
    fn parse_style(&mut self, sy: &mut Style, base: &GridCell, text: &[u8]) -> bool {
        let saved = *sy;
        let text = std::str::from_utf8(text).unwrap();
        for word in text.split([' ', ',', '\n']).filter(|w| !w.is_empty()) {
            let (key, value) = word.split_once('=').unwrap_or((word, ""));
            let ok = match (key, value) {
                ("default", "") => {
                    sy.cell.fg = base.fg;
                    sy.cell.bg = base.bg;
                    sy.cell.us = base.us;
                    sy.cell.attr = base.attr;
                    sy.cell.flags = base.flags;
                    sy.link = 0;
                    true
                }
                ("ignore", "") => {
                    sy.ignore = true;
                    true
                }
                ("noignore", "") => {
                    sy.ignore = false;
                    true
                }
                ("push-default", "") => {
                    sy.default_type = DefaultType::Push;
                    true
                }
                ("pop-default", "") => {
                    sy.default_type = DefaultType::Pop;
                    true
                }
                ("set-default", "") => {
                    sy.default_type = DefaultType::Set;
                    true
                }
                ("nolist", "") => {
                    sy.list = List::Off;
                    true
                }
                ("list", v) => {
                    sy.list = match v {
                        "on" => List::On,
                        "focus" => List::Focus,
                        "left-marker" => List::LeftMarker,
                        "right-marker" => List::RightMarker,
                        _ => return fail(sy, saved),
                    };
                    true
                }
                ("norange", "") => {
                    sy.range_type = RangeType::None;
                    sy.range_argument = 0;
                    sy.range_string = [0; 16];
                    true
                }
                ("range", v) => {
                    let (kind, arg) = v.split_once('|').unwrap_or((v, ""));
                    sy.range_string = [0; 16];
                    sy.range_argument = 0;
                    sy.range_type = match kind {
                        "left" => RangeType::Left,
                        "right" => RangeType::Right,
                        "window" => RangeType::Window,
                        "pane" => RangeType::Pane,
                        "session" => RangeType::Session,
                        "control" => RangeType::Control,
                        "user" => RangeType::User,
                        _ => return fail(sy, saved),
                    };
                    match sy.range_type {
                        RangeType::User => {
                            let n = arg.len().min(15);
                            sy.range_string[..n].copy_from_slice(&arg.as_bytes()[..n]);
                        }
                        RangeType::Window | RangeType::Control => {
                            sy.range_argument = arg.parse().unwrap();
                        }
                        RangeType::Pane | RangeType::Session => {
                            sy.range_argument = arg[1..].parse().unwrap();
                        }
                        _ => {}
                    }
                    true
                }
                ("align", v) => {
                    sy.align = match v {
                        "left" => Align::Left,
                        "centre" => Align::Centre,
                        "right" => Align::Right,
                        "absolute-centre" => Align::AbsoluteCentre,
                        _ => return fail(sy, saved),
                    };
                    true
                }
                ("fill", v) => colour(v).map(|c| sy.fill = c).is_some(),
                ("fg", v) => colour(v)
                    .map(|c| sy.cell.fg = if c == 8 { base.fg } else { c })
                    .is_some(),
                ("bg", v) => colour(v)
                    .map(|c| sy.cell.bg = if c == 8 { base.bg } else { c })
                    .is_some(),
                ("bold", "") => {
                    sy.cell.attr |= GridAttr::GRID_ATTR_BRIGHT;
                    true
                }
                ("link", v) => {
                    sy.link = if v.is_empty() { 0 } else { v.len() as u32 };
                    true
                }
                ("nolink", "") => {
                    sy.link = 0;
                    true
                }
                _ => false,
            };
            if !ok {
                return fail(sy, saved);
            }
        }
        true
    }

    fn link(&mut self, link: u32) -> u32 {
        self.links.push(link);
        100 + link
    }

    fn logging(&self) -> bool {
        self.logging
    }

    fn log(&mut self, message: &str) {
        self.log.push(message.to_string());
    }
}

fn fail(sy: &mut Style, saved: Style) -> bool {
    *sy = saved;
    false
}

/// Draws with ranges and the default cell as base.
fn draw_with(host: &mut Host, available: u32, s: &str) -> Drawn {
    draw(
        host,
        &GRID_DEFAULT_CELL,
        available,
        s.as_bytes(),
        true,
        false,
    )
}

fn drawn(available: u32, s: &str) -> Drawn {
    draw_with(&mut Host::default(), available, s)
}

/// The cells after all puts, by column. Untouched columns are None.
fn row(d: &Drawn, width: u32) -> Vec<Option<GridCell>> {
    let mut out = vec![None; width as usize];
    for put in &d.puts {
        for (i, gc) in put.cells.iter().enumerate() {
            let x = put.x as usize + i;
            if x < out.len() {
                out[x] = Some(*gc);
            }
        }
    }
    out
}

/// The text of the row: untouched columns are `.`, padding is left out.
fn text(d: &Drawn, width: u32) -> String {
    let mut out = String::new();
    for gc in row(d, width) {
        match gc {
            None => out.push('.'),
            Some(gc) if gc.flags.intersects(GridFlag::PADDING) => {}
            Some(gc) => out.push_str(std::str::from_utf8(gc.data.initialized_slice()).unwrap()),
        }
    }
    out
}

/// The foreground colours of the row, `.` for untouched columns.
fn fgs(d: &Drawn, width: u32) -> String {
    row(d, width)
        .iter()
        .map(|gc| match gc {
            None => '.',
            Some(gc) => char::from_digit(gc.fg as u32, 10).unwrap(),
        })
        .collect()
}

fn ranges(d: &Drawn) -> Vec<(RangeType, u32, u32, u32)> {
    d.ranges
        .iter()
        .map(|r| (r.kind, r.argument, r.start, r.end))
        .collect()
}

#[test]
fn alignments() {
    assert_eq!(text(&drawn(10, "abc"), 10), "abc.......");
    assert_eq!(text(&drawn(10, "#[align=right]abc"), 10), ".......abc");
    assert_eq!(text(&drawn(10, "#[align=centre]abcd"), 10), "...abcd...");
    // The centre is centred between the left and the right.
    assert_eq!(
        text(&drawn(12, "ll#[align=centre]cc#[align=right]rrrrrr"), 12),
        "ll.cc.rrrrrr"
    );
    // The absolute centre ignores the left and right.
    assert_eq!(
        text(
            &drawn(12, "ll#[align=absolute-centre]cc#[align=right]rrrrrr"),
            12
        ),
        "ll...ccrrrrr"
    );
    // align= without a list goes back to the left with noalign style.
    assert_eq!(
        text(&drawn(10, "#[align=right]a#[align=left]b"), 10),
        "b........a"
    );
}

#[test]
fn trimming_without_a_list() {
    // The centre goes first, then the right, then the left.
    assert_eq!(
        text(&drawn(10, "llll#[align=centre]cccc#[align=right]rrrr"), 10),
        "llllccrrrr"
    );
    assert_eq!(
        text(&drawn(6, "llll#[align=centre]cccc#[align=right]rrrr"), 6),
        "llllrr"
    );
    assert_eq!(text(&drawn(3, "llll#[align=right]rrrr"), 3), "lll");
    // The centre keeps its middle and the right its end.
    assert_eq!(text(&drawn(2, "#[align=centre]abcd"), 2), "bc");
    assert_eq!(text(&drawn(5, "l#[align=right]abcdef"), 5), "lcdef");
    // Nothing fits.
    assert_eq!(text(&drawn(0, "abc"), 0), "");
}

#[test]
fn wide_characters_are_not_cut() {
    // 日 fits, 本 would need a column more than there is.
    let d = drawn(3, "日本");
    assert_eq!(text(&d, 3), "日.");
    let cells = row(&d, 3);
    assert!(cells[1].unwrap().flags.intersects(GridFlag::PADDING));
    assert_eq!(text(&drawn(4, "日本"), 4), "日本");
}

#[test]
fn zero_width_characters_join() {
    let d = drawn(5, "e\u{301}x");
    assert_eq!(text(&d, 5), "e\u{301}x...");
    // Nothing to join at the start, so it is dropped.
    assert_eq!(text(&drawn(5, "\u{301}x"), 5), "x....");
    // The variation selector makes the cell before two columns wide.
    let d = drawn(5, "a\u{fe0f}b");
    assert_eq!(text(&d, 5), "a\u{fe0f}b..");
    assert!(row(&d, 5)[1].unwrap().flags.intersects(GridFlag::PADDING));
}

#[test]
fn bytes_that_are_not_characters() {
    // Control characters and broken UTF-8 are skipped, the bytes after a
    // broken lead byte are read again.
    assert_eq!(text(&drawn(5, "a\x01b"), 5), "ab...");
    // C1 controls are zero width in tmux-utf8, so they join the cell
    // before (3.8's wcwidth calls them unprintable and skips them).
    assert_eq!(text(&drawn(5, "a\u{85}b"), 5), "a\u{85}b...");
    assert_eq!(
        draw(
            &mut Host::default(),
            &GRID_DEFAULT_CELL,
            5,
            b"\xe6xy\xe6",
            false,
            false
        )
        .puts
        .iter()
        .map(|p| p.cells.len())
        .sum::<usize>(),
        2
    );
    // A NUL ends the text as in C.
    assert_eq!(text(&drawn(5, "ab\0cd"), 5), "ab...");
}

#[test]
fn hashes() {
    assert_eq!(text(&drawn(6, "#"), 6), "#.....");
    assert_eq!(text(&drawn(6, "##"), 6), "#.....");
    assert_eq!(text(&drawn(6, "###"), 6), "##....");
    assert_eq!(text(&drawn(6, "a#b"), 6), "a#b...");
    // An even number before [ escapes the [, an odd one starts a style.
    assert_eq!(text(&drawn(6, "##[x]"), 6), "#[x]..");
    assert_eq!(text(&drawn(6, "###[fg=red]x"), 6), "#x....");
    assert_eq!(fgs(&drawn(6, "###[fg=red]x"), 6), "81....");
}

#[test]
fn styles() {
    let d = drawn(6, "a#[fg=red]b#[bogus]c#[default]d");
    assert_eq!(text(&d, 6), "abcd..");
    assert_eq!(fgs(&d, 6), "8118..");
    // ignore draws styles as text, # sequences before [ are lost.
    assert_eq!(text(&drawn(12, "#[ignore]#[fg=red]x"), 12), "#[fg=red]x..");
    assert_eq!(text(&drawn(12, "#[ignore]##[x"), 12), "x...........");
    assert_eq!(text(&drawn(12, "#[ignore]###[x"), 12), "#[x.........");
    // A style without its ] draws nothing at all.
    let d = drawn(6, "abc#[fg=red");
    assert!(d.puts.is_empty());
    assert!(d.ranges.is_empty());
    assert_eq!(d.fill, None);
}

#[test]
fn default_colours() {
    let mut base = GRID_DEFAULT_CELL;
    base.fg = 3;
    let d = draw(&mut Host::default(), &base, 4, b"a#[fg=red]b", false, true);
    assert_eq!(fgs(&d, 4), "33..");
    let d = draw(&mut Host::default(), &base, 4, b"a#[fg=red]b", false, false);
    assert_eq!(fgs(&d, 4), "31..");
}

#[test]
fn push_pop_and_set_default() {
    // push-default makes the style before it the default, pop-default
    // goes back to the base.
    let d = drawn(
        8,
        "#[fg=red]#[push-default]a#[fg=blue]b#[default]c#[pop-default]#[default]d",
    );
    assert_eq!(fgs(&d, 8), "1418....");
    // set-default also changes what pop-default goes back to.
    let d = drawn(
        8,
        "#[fg=red]#[set-default]a#[fg=blue]b#[default]c#[pop-default]#[default]d",
    );
    assert_eq!(fgs(&d, 8), "1411....");
    // With default_colours the colours set are the base ones again.
    let d = draw(
        &mut Host::default(),
        &GRID_DEFAULT_CELL,
        4,
        b"#[fg=red]#[set-default]a#[fg=blue]b",
        false,
        true,
    );
    assert_eq!(fgs(&d, 4), "88..");
}

#[test]
fn fill() {
    let d = drawn(4, "a#[fill=green]b");
    assert_eq!(d.fill, Some(2));
    assert_eq!(drawn(4, "ab").fill, None);
    // The last fill wins.
    assert_eq!(drawn(4, "#[fill=green]a#[fill=red]b").fill, Some(1));
}

#[test]
fn links() {
    let mut host = Host::default();
    let d = draw_with(&mut host, 6, "a#[link=xyz]b#[nolink]c");
    let cells = row(&d, 6);
    assert_eq!(cells[0].unwrap().link, 0);
    assert_eq!(cells[1].unwrap().link, 103);
    assert_eq!(cells[2].unwrap().link, 0);
    assert_eq!(host.links, vec![3]);
}

#[test]
fn ranges_end_where_the_next_style_starts() {
    // 3.8 fixed the end, which used to be one column too far.
    let d = drawn(
        10,
        "#[range=window|1]ab#[norange]cd#[range=window|2]ef#[norange]",
    );
    assert_eq!(
        ranges(&d),
        vec![(RangeType::Window, 1, 0, 2), (RangeType::Window, 2, 4, 6)]
    );
    // A range still open at the end is dropped.
    assert!(drawn(10, "#[range=left]ab").ranges.is_empty());
    // An empty range is dropped.
    assert!(drawn(10, "#[range=left]#[norange]ab").ranges.is_empty());
    // Styles that keep the range continue it.
    let d = drawn(10, "#[range=user|x]a#[fg=red]b#[range=user|y]c#[norange]");
    assert_eq!(
        ranges(&d),
        vec![(RangeType::User, 0, 0, 2), (RangeType::User, 0, 2, 3)]
    );
    assert_eq!(&d.ranges[0].string[..2], b"x\0");
    // Control ranges continue each other whatever their number, as in C.
    let d = drawn(10, "#[range=control|1]a#[range=control|2]b#[norange]");
    assert_eq!(ranges(&d), vec![(RangeType::Control, 1, 0, 2)]);
    // Without asking for ranges there are none.
    let d = draw(
        &mut Host::default(),
        &GRID_DEFAULT_CELL,
        10,
        b"#[range=left]ab#[norange]",
        false,
        false,
    );
    assert!(d.ranges.is_empty());
}

#[test]
fn ranges_move_with_their_section() {
    let d = drawn(10, "#[align=right range=right]ab#[norange]");
    assert_eq!(text(&d, 10), "........ab");
    assert_eq!(ranges(&d), vec![(RangeType::Right, 0, 8, 10)]);
    // Trimmed with the centre: the right end of the range is cut.
    let d = drawn(4, "#[align=centre range=left]abcdef#[norange]");
    assert_eq!(text(&d, 4), "bcde");
    assert_eq!(ranges(&d), vec![(RangeType::Left, 0, 0, 4)]);
    // A range in what was trimmed away goes.
    let d = drawn(4, "abcd#[align=centre range=left]xy#[norange]");
    assert!(d.ranges.is_empty());
}

/// The window list of 3.8's default status line: left, list, right.
const STATUS: &str = "#[align=left range=left default][s] #[norange default]#[list=on align=left]#[list=left-marker]<#[list=right-marker]>#[list=on]#[range=window|0]0:a #[norange] #[range=window|1 list=focus]1:b*#[norange list=on] #[range=window|2]2:c #[norange]#[nolist align=right range=right]R#[norange default]";

#[test]
fn window_list_fits() {
    let d = drawn(30, STATUS);
    assert_eq!(text(&d, 30), "[s] 0:a  1:b* 2:c ...........R");
    assert_eq!(
        ranges(&d),
        vec![
            (RangeType::Left, 0, 0, 4),
            (RangeType::Window, 0, 4, 8),
            (RangeType::Window, 1, 9, 13),
            (RangeType::Window, 2, 14, 18),
            (RangeType::Right, 0, 29, 30),
        ]
    );
}

#[test]
fn window_list_trimmed_keeps_the_focus() {
    // 10 columns for a 14 column list: trimmed around the focus with
    // both markers.
    let d = drawn(15, STATUS);
    assert_eq!(text(&d, 15), "[s] <  1:b* 2>R");
    assert_eq!(
        ranges(&d),
        vec![
            (RangeType::Left, 0, 0, 4),
            (RangeType::Window, 0, 5, 6),
            (RangeType::Window, 1, 7, 11),
            (RangeType::Window, 2, 12, 13),
            (RangeType::Right, 0, 14, 15),
        ]
    );
    // With two columns only the left marker fits, it needs more room
    // than it takes and the right one does not get it.
    let d = drawn(7, STATUS);
    assert_eq!(text(&d, 7), "[s] <bR");
}

#[test]
fn window_list_without_focus_shows_its_start() {
    let s = STATUS.replace(" list=focus", "");
    let d = drawn(15, &s);
    assert_eq!(text(&d, 15), "[s] 0:a  1:b*>R");
}

#[test]
fn focus_outside_a_list_changes_nothing() {
    // Before the list it is ignored, and the text after it is drawn.
    assert_eq!(text(&drawn(6, "#[list=focus]abc"), 6), "abc...");
    assert_eq!(text(&drawn(6, "#[list=left-marker]abc"), 6), "abc...");
}

#[test]
fn list_alignments() {
    let list = "#[list=on align=ALIGN]#[list=left-marker]<#[list=right-marker]>#[list=on]aa#[list=focus]bb#[list=on]cc#[nolist]";
    let at = |align: &str, w: u32| text(&drawn(w, &list.replace("ALIGN", align)), w);
    assert_eq!(at("left", 10), "aabbcc....");
    assert_eq!(at("centre", 10), "..aabbcc..");
    assert_eq!(at("right", 10), "....aabbcc");
    assert_eq!(at("absolute-centre", 10), "..aabbcc..");
    // Trimmed around the focus.
    assert_eq!(at("left", 4), "<bb>");
    assert_eq!(at("centre", 4), "<bb>");
    // After the list follows it.
    let after = "L#[list=on align=left]xx#[nolist]after";
    assert_eq!(text(&drawn(12, after), 12), "Lxxafter....");
    // A list trimmed away entirely takes what follows it along.
    assert_eq!(
        text(&drawn(3, "LLL#[list=on align=left]xx#[nolist]a"), 3),
        "LLL"
    );
}

#[test]
fn offsets_below_zero_wrap() {
    // The centre before an absolute centre list can start left of the
    // cursor, which C's unsigned sums wrap.
    let d = drawn(
        4,
        "#[align=centre]abcd#[list=on align=absolute-centre]x#[nolist]",
    );
    assert!(d.puts.iter().any(|p| p.x > u32::MAX / 2));
}

#[test]
fn logging() {
    let mut host = Host {
        logging: true,
        ..Host::default()
    };
    draw_with(
        &mut host,
        10,
        "a#[bogus]#[range=left align=right]b#[norange]",
    );
    assert_eq!(
        host.log[0],
        "format_draw: a#[bogus]#[range=left align=right]b#[norange]"
    );
    assert!(
        host.log
            .contains(&"format_draw: invalid style 'bogus'".to_string())
    );
    assert!(
        host.log
            .contains(&"format_draw: change LEFT -> RIGHT".to_string())
    );
    assert!(
        host.log
            .contains(&"format_draw: range left at 9-10".to_string())
    );
    // Nothing is logged unless asked.
    let mut host = Host::default();
    draw_with(&mut host, 10, "a#[bogus]");
    assert!(host.log.is_empty());
}
