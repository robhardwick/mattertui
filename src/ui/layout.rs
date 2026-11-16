//! Auto-layout: rooms and devices in declaration order → rectangles that fill
//! the screen in both directions, in the style of btop.
//!
//! - Rooms are boxes, packed masonry-style into as many columns as fit (each
//!   into the shortest column), then stretched so every column reaches the
//!   bottom. Spare rows go to rooms by how many graphs they hold.
//! - Inside a room, every device is its own box; boxes stretch to fill the
//!   room, so taller terminals mean taller graphs.
//! - A **featured** device fills its room like btop's CPU box: its graph
//!   takes the whole room and the other devices float over its bottom-right
//!   as boxes at their natural height, like btop's per-core panel.
//! - If the minimum sizes don't fit, featured rooms fall back to plain boxes,
//!   then devices become one-line rows inside their room box (like btop's
//!   core list), and finally the caller scrolls.

use ratatui::layout::Rect;

use crate::config::DeviceKind;

pub const MIN_ROOM_WIDTH: u16 = 38;
/// Minimum width of a device box; two side by side need twice this.
const MIN_BOX_WIDTH: u16 = 34;
/// Inner width a featured room needs to float other devices over the graph.
const FEATURED_SIDE_AT: u16 = 64;
/// Graph left showing to the right of floating boxes…
const FLOAT_MARGIN_X: u16 = 3;
/// …and at least this much above and below them.
const FLOAT_MARGIN_Y: u16 = 1;

#[derive(Debug, Clone)]
pub struct RoomSpec {
    pub tiles: Vec<TileSpec>,
}

#[derive(Debug, Clone, Copy)]
pub struct TileSpec {
    pub kind: DeviceKind,
    pub featured: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// One line inside the room box: name, mini graph, value.
    Row,
    /// The device's own box.
    Boxed,
    /// Fills the room (left of any side column), no box of its own.
    Featured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Density {
    Full,
    NoFeatured,
    Rows,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedRoom {
    pub room: usize,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedTile {
    pub room: usize,
    pub tile: usize,
    pub rect: Rect,
    pub variant: Variant,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub rooms: Vec<PlacedRoom>,
    pub tiles: Vec<PlacedTile>,
    /// Total height used; larger than the area when the view must scroll.
    pub height: u16,
}

/// Info rows a device box always shows (more appear when there's room).
pub fn min_info_rows(kind: DeviceKind) -> u16 {
    match kind {
        DeviceKind::Plug | DeviceKind::LeakSensor => 1,
        DeviceKind::TemperatureSensor => 2,
        DeviceKind::Thermostat => 3,
    }
}

/// Minimum rows for a device in a variant (boxes include their border).
pub fn min_height(kind: DeviceKind, variant: Variant) -> u16 {
    match variant {
        Variant::Row => 1,
        // border + info rows + one graph row
        Variant::Boxed => 2 + min_info_rows(kind) + 1,
        // info line + gauge/meter line + three graph rows
        Variant::Featured => 5,
    }
}

/// Height a box floating over a featured graph takes: all its info rows and
/// a short graph. Floating boxes don't stretch.
pub fn natural_height(kind: DeviceKind) -> u16 {
    match kind {
        DeviceKind::Plug => 2 + 2 + 1,
        // A wet/dry history needs only one row.
        DeviceKind::LeakSensor => 2 + 3 + 1,
        DeviceKind::TemperatureSensor => 2 + 3 + 2,
        DeviceKind::Thermostat => 2 + 5 + 2,
    }
}

/// Lay out `rooms` within `area`. Rects are absolute (offset by `area`).
pub fn layout(rooms: &[RoomSpec], area: Rect) -> Layout {
    if rooms.is_empty() || area.width == 0 {
        return Layout::default();
    }
    for density in [Density::Full, Density::NoFeatured, Density::Rows] {
        let plan = plan(rooms, area, density);
        if plan.tallest() <= area.height {
            return plan.place(rooms, area, density, true);
        }
    }
    plan(rooms, area, Density::Rows).place(rooms, area, Density::Rows, false)
}

/// Rooms assigned to columns, with column positions and minimum heights.
struct Plan {
    /// Per column: (x, width).
    spans: Vec<(u16, u16)>,
    /// Per column: (room index, min height).
    cols: Vec<Vec<(usize, u16)>>,
}

impl Plan {
    fn tallest(&self) -> u16 {
        self.cols.iter().map(|c| c.iter().map(|(_, h)| h).sum::<u16>()).max().unwrap_or(0)
    }

    /// Turn the plan into rectangles; `stretch` fills each column's height.
    fn place(&self, rooms: &[RoomSpec], area: Rect, density: Density, stretch: bool) -> Layout {
        let mut out = Layout::default();
        for (c, col) in self.cols.iter().enumerate() {
            let (x, width) = self.spans[c];
            let mins: Vec<u16> = col.iter().map(|(_, h)| *h).collect();
            let weights: Vec<u16> = col.iter().map(|(r, _)| room_weight(&rooms[*r], density)).collect();
            let heights = if stretch { grow(&mins, &weights, area.height) } else { mins };
            let mut y = area.y;
            for (&(r, _), &h) in col.iter().zip(&heights) {
                let rect = Rect::new(x, y, width, h);
                out.rooms.push(PlacedRoom { room: r, rect });
                out.tiles.extend(place_room(r, &rooms[r], rect, density));
                y += h;
            }
            out.height = out.height.max(y - area.y);
        }
        out.rooms.sort_by_key(|r| r.room);
        out
    }
}

/// Rooms that aren't wide stack this many to a column beside wide ones.
const STACK_BESIDE_WIDE: usize = 2;

fn plan(rooms: &[RoomSpec], area: Rect, density: Density) -> Plan {
    let max_columns = (area.width / MIN_ROOM_WIDTH).clamp(1, rooms.len() as u16) as usize;
    let (wide, narrow): (Vec<usize>, Vec<usize>) = (0..rooms.len()).partition(|&r| wants_width(&rooms[r], density));
    let narrow_columns = narrow.len().div_ceil(STACK_BESIDE_WIDE);
    let mut cols: Vec<Vec<(usize, u16)>> = if !wide.is_empty() && wide.len() + narrow_columns <= max_columns {
        // Each wide room gets a column of its own; the rest stack beside
        // them, two to a column, so they don't squeeze the wide ones.
        let mut cols: Vec<_> = wide.iter().map(|&r| vec![(r, 0)]).collect();
        cols.extend(pack(rooms, &narrow, narrow_columns, area, wide.len() + narrow_columns, density));
        cols.sort_by_key(|c| c[0].0);
        cols
    } else {
        pack(rooms, &(0..rooms.len()).collect::<Vec<_>>(), max_columns, area, max_columns, density)
    };
    // Like btop's CPU box, a column holding a featured room (graph plus a
    // side column) gets double width.
    let weights: Vec<u16> = cols
        .iter()
        .map(|col| if col.iter().any(|(r, _)| wants_width(&rooms[*r], density)) { 2 } else { 1 })
        .collect();
    let spans = weighted_spans(area.x, area.width, &weights);
    // Re-measure at the final widths so the fit check stays honest.
    for (col, &(_, width)) in cols.iter_mut().zip(&spans) {
        for (r, h) in col.iter_mut() {
            *h = room_min_height(&rooms[*r], width, density);
        }
    }
    Plan { spans, cols }
}

/// Masonry: each room (in order) into the shortest of `n` columns, leftmost
/// on ties, measured as if the area held `of` equal columns.
fn pack(rooms: &[RoomSpec], which: &[usize], n: usize, area: Rect, of: usize, density: Density) -> Vec<Vec<(usize, u16)>> {
    let mut cols: Vec<Vec<(usize, u16)>> = vec![Vec::new(); n];
    let (_, width) = span(area.x, area.width, of as u16, 0);
    let height = |c: &Vec<(usize, u16)>| c.iter().map(|(_, h)| h).sum::<u16>();
    for &r in which {
        let c = (0..n).min_by_key(|&c| (height(&cols[c]), c)).unwrap_or(0);
        cols[c].push((r, room_min_height(&rooms[r], width, density)));
    }
    cols.retain(|c| !c.is_empty());
    cols
}

fn wants_width(room: &RoomSpec, density: Density) -> bool {
    density == Density::Full && room.tiles.iter().any(|t| t.featured) && room.tiles.len() > 1
}

/// Split a span by weights, giving the rounding remainder to the first ones.
fn weighted_spans(start: u16, len: u16, weights: &[u16]) -> Vec<(u16, u16)> {
    let total: u32 = weights.iter().map(|&w| u32::from(w)).sum();
    let mut widths: Vec<u16> = weights.iter().map(|&w| (u32::from(len) * u32::from(w) / total) as u16).collect();
    let mut rest = len - widths.iter().sum::<u16>();
    for w in widths.iter_mut() {
        if rest == 0 {
            break;
        }
        *w += 1;
        rest -= 1;
    }
    let mut x = start;
    widths
        .into_iter()
        .map(|w| {
            let s = (x, w);
            x += w;
            s
        })
        .collect()
}

/// Split a span into `n` equal parts, giving the remainder to the first ones.
fn span(start: u16, len: u16, n: u16, i: u16) -> (u16, u16) {
    let base = len / n;
    let extra = len % n;
    (start + i * base + i.min(extra), base + u16::from(i < extra))
}

/// How a room arranges its devices at a density.
enum Shape {
    /// Device rows, one per line.
    Rows,
    /// Device boxes in `n` stacked columns.
    Boxes { columns: Vec<Vec<usize>> },
    /// A featured device filling the room, with others in a side column.
    Featured { featured: usize, side: Vec<usize> },
}

fn inner_width(room_width: u16) -> u16 {
    room_width.saturating_sub(2)
}

fn shape(room: &RoomSpec, room_width: u16, density: Density) -> Shape {
    if density == Density::Rows {
        return Shape::Rows;
    }
    let inner = inner_width(room_width);
    if density == Density::Full
        && let Some(f) = room.tiles.iter().position(|t| t.featured)
    {
        let others: Vec<usize> = (0..room.tiles.len()).filter(|&i| i != f).collect();
        if others.is_empty() || inner >= FEATURED_SIDE_AT {
            return Shape::Featured { featured: f, side: others };
        }
    }
    let n = if inner >= MIN_BOX_WIDTH * 2 && room.tiles.len() > 1 { 2 } else { 1 };
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut heights = vec![0u16; n];
    for (i, t) in room.tiles.iter().enumerate() {
        let c = (0..n).min_by_key(|&c| (heights[c], c)).unwrap_or(0);
        heights[c] += min_height(t.kind, Variant::Boxed);
        columns[c].push(i);
    }
    Shape::Boxes { columns }
}

fn boxes_height(room: &RoomSpec, tiles: &[usize]) -> u16 {
    tiles.iter().map(|&i| min_height(room.tiles[i].kind, Variant::Boxed)).sum()
}

fn room_min_height(room: &RoomSpec, width: u16, density: Density) -> u16 {
    let inner = match shape(room, width, density) {
        Shape::Rows => room.tiles.len() as u16,
        Shape::Boxes { columns } => columns.iter().map(|c| boxes_height(room, c)).max().unwrap_or(0),
        Shape::Featured { featured, side } => {
            // Floating boxes keep a row of graph above and below them.
            let floating = if side.is_empty() { 0 } else { boxes_height(room, &side) + 2 * FLOAT_MARGIN_Y };
            min_height(room.tiles[featured].kind, Variant::Featured).max(floating)
        }
    };
    inner.max(1) + 2
}

/// Share of spare column height a room takes: its graphs.
fn room_weight(room: &RoomSpec, density: Density) -> u16 {
    if density == Density::Rows {
        return 0;
    }
    room.tiles.iter().map(|t| if t.featured && density == Density::Full { 2 } else { 1 }).sum()
}

/// Grow `mins` to total `target` (if larger), sharing the extra by `weights`
/// (equally if they're all zero).
fn grow(mins: &[u16], weights: &[u16], target: u16) -> Vec<u16> {
    let total: u16 = mins.iter().sum();
    let extra = target.saturating_sub(total);
    if extra == 0 || mins.is_empty() {
        return mins.to_vec();
    }
    let weights: Vec<u32> = if weights.iter().any(|&w| w > 0) {
        weights.iter().map(|&w| u32::from(w)).collect()
    } else {
        vec![1; mins.len()]
    };
    let sum: u32 = weights.iter().sum();
    let mut out: Vec<u16> = mins.to_vec();
    let mut given = 0u16;
    for (o, w) in out.iter_mut().zip(&weights) {
        let share = (u32::from(extra) * w / sum) as u16;
        *o += share;
        given += share;
    }
    // Hand out the rounding remainder to the heaviest first.
    let mut order: Vec<usize> = (0..out.len()).filter(|&i| weights[i] > 0).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(weights[i]));
    for &i in order.iter().cycle().take((extra - given) as usize) {
        out[i] += 1;
    }
    out
}

/// Stack device boxes for `tiles` in `area`, stretched to its height.
fn stack(room_idx: usize, room: &RoomSpec, tiles: &[usize], area: Rect, out: &mut Vec<PlacedTile>) {
    let mins: Vec<u16> = tiles.iter().map(|&i| min_height(room.tiles[i].kind, Variant::Boxed)).collect();
    let heights = grow(&mins, &vec![1; tiles.len()], area.height);
    let mut y = area.y;
    for (&i, &h) in tiles.iter().zip(&heights) {
        out.push(PlacedTile {
            room: room_idx,
            tile: i,
            rect: Rect::new(area.x, y, area.width, h),
            variant: Variant::Boxed,
        });
        y += h;
    }
}

fn place_room(room_idx: usize, room: &RoomSpec, rect: Rect, density: Density) -> Vec<PlacedTile> {
    let inner = Rect::new(rect.x + 1, rect.y + 1, rect.width.saturating_sub(2), rect.height.saturating_sub(2));
    let mut out = Vec::new();
    match shape(room, rect.width, density) {
        Shape::Rows => {
            for (i, _) in room.tiles.iter().enumerate() {
                let y = inner.y + i as u16;
                if y < inner.bottom() {
                    out.push(PlacedTile {
                        room: room_idx,
                        tile: i,
                        rect: Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
                        variant: Variant::Row,
                    });
                }
            }
        }
        Shape::Boxes { columns } => {
            let n = columns.len() as u16;
            for (c, tiles) in columns.iter().enumerate() {
                let (x, w) = span(inner.x, inner.width, n, c as u16);
                stack(room_idx, room, tiles, Rect::new(x, inner.y, w, inner.height), &mut out);
            }
        }
        Shape::Featured { featured, side } => {
            // The graph spans the room; other devices float over it near the
            // right, sitting at the bottom with graph showing on every side
            // (like btop's per-core box). Natural height, or minimum if tight.
            out.push(PlacedTile {
                room: room_idx,
                tile: featured,
                rect: Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height),
                variant: Variant::Featured,
            });
            if !side.is_empty() {
                let side_w = (inner.width * 2 / 5).clamp(MIN_BOX_WIDTH, 48);
                let natural: u16 = side.iter().map(|&i| natural_height(room.tiles[i].kind)).sum();
                let fits = natural + 2 * FLOAT_MARGIN_Y <= inner.height;
                let heights: Vec<u16> = side
                    .iter()
                    .map(|&i| {
                        let kind = room.tiles[i].kind;
                        if fits { natural_height(kind) } else { min_height(kind, Variant::Boxed) }
                    })
                    .collect();
                let total: u16 = heights.iter().sum();
                let x = inner.right().saturating_sub(side_w + FLOAT_MARGIN_X);
                let mut y = inner.bottom().saturating_sub(total + FLOAT_MARGIN_Y).max(inner.y);
                for (&i, &h) in side.iter().zip(&heights) {
                    out.push(PlacedTile {
                        room: room_idx,
                        tile: i,
                        rect: Rect::new(x, y, side_w, h),
                        variant: Variant::Boxed,
                    });
                    y += h;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use DeviceKind::*;

    fn room(kinds: &[(DeviceKind, bool)]) -> RoomSpec {
        RoomSpec {
            tiles: kinds.iter().map(|&(kind, featured)| TileSpec { kind, featured }).collect(),
        }
    }

    fn house() -> Vec<RoomSpec> {
        vec![
            room(&[(Plug, false), (Thermostat, true), (TemperatureSensor, false)]),
            room(&[(TemperatureSensor, false), (Plug, false)]),
            room(&[(Plug, false)]),
            room(&[(TemperatureSensor, true)]),
        ]
    }

    /// Render a layout as ASCII so snapshots show the geometry: room edges
    /// are letters, boxes are `+`-cornered, featured areas `#`, rows `-`.
    fn sketch(l: &Layout, area: Rect) -> String {
        let h = l.height.max(area.height);
        let mut grid = vec![vec![' '; area.width as usize]; h as usize];
        let edge = |r: Rect, c: char, grid: &mut Vec<Vec<char>>| {
            for y in r.top()..r.bottom() {
                for x in r.left()..r.right() {
                    if y == r.top() || y == r.bottom() - 1 || x == r.left() || x == r.right() - 1 {
                        grid[y as usize][x as usize] = c;
                    }
                }
            }
        };
        for r in &l.rooms {
            edge(r.rect, char::from(b'A' + r.room as u8), &mut grid);
        }
        for t in &l.tiles {
            match t.variant {
                Variant::Boxed => edge(t.rect, '+', &mut grid),
                Variant::Featured | Variant::Row => {
                    let c = if t.variant == Variant::Row { '-' } else { '#' };
                    for y in t.rect.top()..t.rect.bottom() {
                        for x in t.rect.left()..t.rect.right() {
                            grid[y as usize][x as usize] = c;
                        }
                    }
                }
            }
            let (x, y) = if t.variant == Variant::Boxed { (t.rect.x + 1, t.rect.y + 1) } else { (t.rect.x, t.rect.y) };
            grid[y as usize][x as usize] = char::from(b'0' + t.tile as u8);
        }
        grid.into_iter()
            .map(|r| r.into_iter().collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assert_sane(l: &Layout, area: Rect) {
        for (i, a) in l.rooms.iter().enumerate() {
            for b in &l.rooms[i + 1..] {
                assert!(!a.rect.intersects(b.rect), "rooms overlap: {a:?} {b:?}");
            }
        }
        for (i, a) in l.tiles.iter().enumerate() {
            for b in &l.tiles[i + 1..] {
                // Boxes float over their room's featured graph by design.
                let floating = a.room == b.room && (a.variant == Variant::Featured || b.variant == Variant::Featured);
                assert!(floating || !a.rect.intersects(b.rect), "tiles overlap: {a:?} {b:?}");
            }
            let room = l.rooms.iter().find(|r| r.room == a.room).unwrap().rect;
            assert!(a.rect.x > room.x && a.rect.right() < room.right(), "tile outside room horizontally: {a:?} in {room:?}");
            assert!(a.rect.y > room.y && a.rect.bottom() < room.bottom(), "tile outside room vertically: {a:?} in {room:?}");
        }
        assert!(l.rooms.iter().all(|r| r.rect.x >= area.x && r.rect.right() <= area.right()));
    }

    #[test]
    fn wide_terminal_fills_every_column() {
        let area = Rect::new(0, 0, 150, 40);
        let l = layout(&house(), area);
        insta::assert_snapshot!(sketch(&l, area));
        assert_sane(&l, area);
        assert_eq!(l.height, area.height);
        let columns: std::collections::BTreeSet<u16> = l.rooms.iter().map(|r| r.rect.x).collect();
        for x in columns {
            let bottom = l.rooms.iter().filter(|r| r.rect.x == x).map(|r| r.rect.bottom()).max().unwrap();
            assert_eq!(bottom, area.bottom(), "column at x={x} doesn't reach the bottom");
        }
    }

    #[test]
    fn featured_room_puts_others_in_a_side_column() {
        let area = Rect::new(0, 0, 100, 30);
        let l = layout(&[house()[0].clone()], area);
        insta::assert_snapshot!(sketch(&l, area));
        let featured = l.tiles.iter().find(|t| t.variant == Variant::Featured).unwrap();
        let others: Vec<_> = l.tiles.iter().filter(|t| t.variant == Variant::Boxed).collect();
        assert_eq!(others.len(), 2);
        assert_eq!(featured.rect.height, area.height - 2, "graph fills the room's height");
        // Graph shows on every side of the floating boxes.
        for t in &others {
            assert!(t.rect.x > featured.rect.x, "graph to the left");
            assert!(t.rect.right() < featured.rect.right(), "graph to the right");
        }
        let top = others.iter().map(|t| t.rect.y).min().unwrap();
        let bottom = others.iter().map(|t| t.rect.bottom()).max().unwrap();
        assert!(top > featured.rect.y && bottom < featured.rect.bottom(), "graph above and below");
        assert_eq!(featured.rect.bottom() - bottom, FLOAT_MARGIN_Y, "bottom-aligned with padding");
        // Natural height, not stretched to fill the room.
        assert_eq!(others[0].rect.height, natural_height(Plug));
        assert_eq!(others[1].rect.height, natural_height(TemperatureSensor));
    }

    #[test]
    fn taller_terminal_means_taller_boxes() {
        let short = layout(&house(), Rect::new(0, 0, 150, 30));
        let tall = layout(&house(), Rect::new(0, 0, 150, 60));
        let h = |l: &Layout, room, tile| l.tiles.iter().find(|t| t.room == room && t.tile == tile).unwrap().rect.height;
        assert!(h(&tall, 0, 1) > h(&short, 0, 1), "featured grows");
        assert!(h(&tall, 1, 0) > h(&short, 1, 0), "boxes grow");
        assert_eq!(h(&tall, 0, 0), h(&short, 0, 0), "floating boxes don't");
    }

    #[test]
    fn standard_terminal_fits() {
        let area = Rect::new(0, 0, 80, 24);
        let l = layout(&house(), area);
        insta::assert_snapshot!(sketch(&l, area));
        assert_sane(&l, area);
        assert_eq!(l.height, area.height);
    }

    #[test]
    fn tiny_terminal_uses_rows_then_scrolls() {
        let area = Rect::new(0, 0, 40, 10);
        let l = layout(&house(), area);
        assert!(l.height > area.height);
        assert!(l.tiles.iter().all(|t| t.variant == Variant::Row));
        assert_sane(&l, area);
    }

    #[test]
    fn sane_at_many_sizes() {
        for (w, h) in [(80, 24), (120, 40), (200, 60), (38, 100), (60, 30), (240, 20), (100, 50)] {
            let area = Rect::new(0, 0, w, h);
            assert_sane(&layout(&house(), area), area);
        }
    }

    #[test]
    fn respects_area_offset() {
        let l = layout(&house(), Rect::new(5, 3, 120, 40));
        assert!(l.rooms.iter().all(|r| r.rect.x >= 5 && r.rect.y >= 3));
        assert_eq!(l.rooms.iter().map(|r| r.rect.bottom()).max(), Some(43));
    }

    #[test]
    fn featured_columns_get_more_width() {
        let area = Rect::new(0, 0, 200, 40);
        let l = layout(&house(), area);
        let living = l.rooms.iter().find(|r| r.room == 0).unwrap().rect.width;
        let bedroom = l.rooms.iter().find(|r| r.room == 1).unwrap().rect.width;
        assert!(living > bedroom, "{living} vs {bedroom}");
        let featured = l.tiles.iter().find(|t| t.room == 0 && t.variant == Variant::Featured).unwrap();
        assert!(featured.rect.width >= 40, "graph gets room: {}", featured.rect.width);
        assert_eq!(weighted_spans(0, 10, &[3, 2]), vec![(0, 6), (6, 4)]);
        assert_eq!(weighted_spans(0, 11, &[1, 1]), vec![(0, 6), (6, 5)]);
    }

    /// Two featured rooms with side devices take a column each; two featured
    /// rooms on their own stack in a third.
    #[test]
    fn lone_featured_rooms_stack_beside_wide_ones() {
        let rooms = [
            room(&[(Thermostat, true), (Plug, false)]),
            room(&[(Thermostat, true), (TemperatureSensor, false)]),
            room(&[(TemperatureSensor, true)]),
            room(&[(TemperatureSensor, true)]),
        ];
        let area = Rect::new(0, 0, 220, 50);
        let l = layout(&rooms, area);
        insta::assert_snapshot!(sketch(&l, area));
        assert_sane(&l, area);
        let rect = |r: usize| l.rooms[r].rect;
        assert!(rect(0).right() <= rect(1).x, "first two side by side");
        assert!(rect(1).right() <= rect(2).x, "third column to their right");
        assert_eq!(rect(2).x, rect(3).x, "last two share a column");
        assert_eq!(rect(2).bottom(), rect(3).y, "stacked");
        assert_eq!(rect(3).bottom(), area.bottom());
    }

    #[test]
    fn grow_shares_by_weight() {
        assert_eq!(grow(&[3, 3], &[1, 3], 10), vec![4, 6]);
        assert_eq!(grow(&[3, 3], &[0, 0], 10), vec![5, 5]);
        assert_eq!(grow(&[5], &[1], 3), vec![5]);
    }
}
