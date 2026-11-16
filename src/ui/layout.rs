//! Auto-layout: rooms and devices in declaration order → rectangles that fill
//! the screen in both directions.
//!
//! 1. Rooms are packed masonry-style into as many columns as fit, each going
//!    into the currently shortest column (by minimum height).
//! 2. If the minimum heights don't fit, tiles shrink (featured → normal →
//!    compact); if even compact doesn't fit, the caller scrolls.
//! 3. When they fit, each column is stretched to the full height: spare rows
//!    go to rooms in proportion to how many graphs they hold, and within a
//!    room to its graph tiles, so a taller terminal means taller graphs.
//!
//! Tiles are borderless sections inside their room's box: a header row, the
//! readouts, then (for sensors and thermostats) a graph that absorbs spare
//! rows.

use ratatui::layout::Rect;

use crate::config::DeviceKind;

pub const MIN_ROOM_WIDTH: u16 = 38;
/// Inner width at which a room splits its tiles into two columns.
const TWO_INNER_COLUMNS_AT: u16 = 76;

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
    /// One line.
    Compact,
    Normal,
    /// Big readout and a larger graph.
    Featured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Density {
    Full,
    NoFeatured,
    Compact,
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

/// Minimum rows for a tile, including its header row.
pub fn min_height(kind: DeviceKind, variant: Variant) -> u16 {
    match (variant, kind) {
        (Variant::Compact, _) => 1,
        // header, state, timeline
        (_, DeviceKind::Plug) => 3,
        // header, readouts, graph ≥ 1
        (Variant::Normal, DeviceKind::TemperatureSensor) => 3,
        // header, readouts, setpoint/status, graph ≥ 1
        (Variant::Normal, DeviceKind::Thermostat) => 4,
        // header, big readout (4), gauge/info, graph ≥ 2
        (Variant::Featured, _) => 8,
    }
}

/// Share of spare rows a tile takes (its graph grows; plugs don't).
fn flex(kind: DeviceKind, variant: Variant) -> u16 {
    match (variant, kind) {
        (Variant::Compact, _) => 0,
        (Variant::Featured, DeviceKind::Plug) | (_, DeviceKind::Plug) => 1,
        (Variant::Featured, _) => 2,
        (Variant::Normal, _) => 1,
    }
}

fn variant(spec: TileSpec, density: Density) -> Variant {
    match density {
        Density::Compact => Variant::Compact,
        Density::NoFeatured => Variant::Normal,
        Density::Full if spec.featured => Variant::Featured,
        Density::Full => Variant::Normal,
    }
}

/// Lay out `rooms` within `area`. Rects are absolute (offset by `area`).
pub fn layout(rooms: &[RoomSpec], area: Rect) -> Layout {
    if rooms.is_empty() || area.width == 0 {
        return Layout::default();
    }
    for density in [Density::Full, Density::NoFeatured, Density::Compact] {
        let plan = plan(rooms, area, density);
        if plan.tallest() <= area.height {
            return plan.place(rooms, area, density, true);
        }
    }
    plan(rooms, area, Density::Compact).place(rooms, area, Density::Compact, false)
}

/// Rooms assigned to columns, with minimum heights.
struct Plan {
    columns: u16,
    /// Per column: (room index, min height).
    cols: Vec<Vec<(usize, u16)>>,
}

impl Plan {
    fn tallest(&self) -> u16 {
        self.cols
            .iter()
            .map(|c| c.iter().map(|(_, h)| h).sum::<u16>())
            .max()
            .unwrap_or(0)
    }

    /// Turn the plan into rectangles; `stretch` fills each column's height.
    fn place(&self, rooms: &[RoomSpec], area: Rect, density: Density, stretch: bool) -> Layout {
        let mut out = Layout::default();
        for (c, col) in self.cols.iter().enumerate() {
            let (x, width) = column_span(area, self.columns, c as u16);
            let mins: Vec<u16> = col.iter().map(|(_, h)| *h).collect();
            let weights: Vec<u16> = col.iter().map(|(r, _)| room_weight(&rooms[*r], width, density)).collect();
            let heights = if stretch {
                grow(&mins, &weights, area.height, true)
            } else {
                mins
            };
            let mut y = area.y;
            for (&(r, _), &h) in col.iter().zip(&heights) {
                let rect = Rect::new(x, y, width, h);
                out.rooms.push(PlacedRoom { room: r, rect });
                out.tiles.extend(place_tiles(r, &rooms[r], rect, density));
                y += h;
            }
            out.height = out.height.max(y - area.y);
        }
        out.rooms.sort_by_key(|r| r.room);
        out
    }
}

fn plan(rooms: &[RoomSpec], area: Rect, density: Density) -> Plan {
    let columns = (area.width / MIN_ROOM_WIDTH).clamp(1, rooms.len() as u16);
    let mut cols: Vec<Vec<(usize, u16)>> = vec![Vec::new(); columns as usize];
    let height = |c: &Vec<(usize, u16)>| c.iter().map(|(_, h)| h).sum::<u16>();
    for (r, room) in rooms.iter().enumerate() {
        // Shortest column, leftmost on ties.
        let c = (0..columns as usize).min_by_key(|&c| (height(&cols[c]), c)).unwrap_or(0);
        let (_, width) = column_span(area, columns, c as u16);
        cols[c].push((r, room_min_height(room, width, density)));
    }
    Plan { columns, cols }
}

/// Split `area` into `columns` equal columns, giving the remainder to the
/// leftmost ones.
fn column_span(area: Rect, columns: u16, col: u16) -> (u16, u16) {
    let base = area.width / columns;
    let extra = area.width % columns;
    let x = area.x + col * base + col.min(extra);
    let w = base + u16::from(col < extra);
    (x, w)
}

/// The tiles of a room split into inner columns. Rooms with a featured tile
/// stay single-column so the featured tile spans the room.
fn inner_columns(room: &RoomSpec, room_width: u16, density: Density) -> Vec<Vec<usize>> {
    let inner = room_width.saturating_sub(4);
    let has_featured = room.tiles.iter().any(|t| variant(*t, density) == Variant::Featured);
    let n = if inner >= TWO_INNER_COLUMNS_AT && !has_featured && room.tiles.len() > 1 { 2 } else { 1 };
    let mut cols: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut heights = vec![0u16; n];
    for (i, t) in room.tiles.iter().enumerate() {
        let c = (0..n).min_by_key(|&c| (heights[c], c)).unwrap_or(0);
        heights[c] += min_height(t.kind, variant(*t, density));
        cols[c].push(i);
    }
    cols
}

fn room_min_height(room: &RoomSpec, width: u16, density: Density) -> u16 {
    let cols = inner_columns(room, width, density);
    let inner = cols
        .iter()
        .map(|c| c.iter().map(|&i| min_height(room.tiles[i].kind, variant(room.tiles[i], density))).sum::<u16>())
        .max()
        .unwrap_or(0)
        .max(1);
    inner + 2
}

fn room_weight(room: &RoomSpec, width: u16, density: Density) -> u16 {
    inner_columns(room, width, density)
        .iter()
        .map(|c| c.iter().map(|&i| flex(room.tiles[i].kind, variant(room.tiles[i], density))).sum::<u16>())
        .max()
        .unwrap_or(0)
}

/// Grow `mins` to total `target` (if larger), sharing the extra by `weights`.
/// With all-zero weights the extra is shared equally if `even_if_rigid`, else
/// left unassigned.
fn grow(mins: &[u16], weights: &[u16], target: u16, even_if_rigid: bool) -> Vec<u16> {
    let total: u16 = mins.iter().sum();
    let extra = target.saturating_sub(total);
    if extra == 0 || mins.is_empty() {
        return mins.to_vec();
    }
    let weights: Vec<u32> = if weights.iter().any(|&w| w > 0) {
        weights.iter().map(|&w| u32::from(w)).collect()
    } else if even_if_rigid {
        vec![1; mins.len()]
    } else {
        return mins.to_vec();
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

fn place_tiles(room_idx: usize, room: &RoomSpec, rect: Rect, density: Density) -> Vec<PlacedTile> {
    // One cell of padding inside the border on each side.
    let inner = Rect::new(rect.x + 2, rect.y + 1, rect.width.saturating_sub(4), rect.height.saturating_sub(2));
    let cols = inner_columns(room, rect.width, density);
    let n = cols.len() as u16;
    let mut out = Vec::new();
    for (c, tiles) in cols.iter().enumerate() {
        // Inner columns get a 2-cell gutter between them.
        let (x, w) = if n > 1 {
            let (x, w) = column_span(Rect::new(inner.x, inner.y, inner.width + 2, inner.height), n, c as u16);
            (x, w.saturating_sub(2))
        } else {
            (inner.x, inner.width)
        };
        let variants: Vec<Variant> = tiles.iter().map(|&i| variant(room.tiles[i], density)).collect();
        let mins: Vec<u16> = tiles.iter().zip(&variants).map(|(&i, &v)| min_height(room.tiles[i].kind, v)).collect();
        let weights: Vec<u16> = tiles.iter().zip(&variants).map(|(&i, &v)| flex(room.tiles[i].kind, v)).collect();
        let heights = grow(&mins, &weights, inner.height, false);
        let mut y = inner.y;
        for ((&i, &v), &h) in tiles.iter().zip(&variants).zip(&heights) {
            out.push(PlacedTile {
                room: room_idx,
                tile: i,
                rect: Rect::new(x, y, w, h),
                variant: v,
            });
            y += h;
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
            room(&[(TemperatureSensor, false)]),
        ]
    }

    /// Render a layout as ASCII so snapshots show the geometry.
    fn sketch(l: &Layout, area: Rect) -> String {
        let h = l.height.max(area.height);
        let mut grid = vec![vec![' '; area.width as usize]; h as usize];
        for r in &l.rooms {
            for y in r.rect.top()..r.rect.bottom() {
                for x in r.rect.left()..r.rect.right() {
                    let edge = y == r.rect.top() || y == r.rect.bottom() - 1 || x == r.rect.left() || x == r.rect.right() - 1;
                    if edge {
                        grid[y as usize][x as usize] = char::from(b'A' + r.room as u8);
                    }
                }
            }
        }
        for t in &l.tiles {
            let c = match t.variant {
                Variant::Compact => '-',
                Variant::Normal => '.',
                Variant::Featured => '#',
            };
            for y in t.rect.top()..t.rect.bottom() {
                for x in t.rect.left()..t.rect.right() {
                    grid[y as usize][x as usize] = c;
                }
            }
            grid[t.rect.y as usize][t.rect.x as usize] = char::from(b'0' + t.tile as u8);
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
                assert!(!a.rect.intersects(b.rect), "tiles overlap: {a:?} {b:?}");
            }
            let room = l.rooms.iter().find(|r| r.room == a.room).unwrap().rect;
            assert!(a.rect.x > room.x && a.rect.right() < room.right(), "tile outside room horizontally");
            assert!(a.rect.y > room.y && a.rect.bottom() < room.bottom(), "tile outside room vertically");
        }
        assert!(l.rooms.iter().all(|r| r.rect.x >= area.x && r.rect.right() <= area.right()));
    }

    #[test]
    fn wide_terminal_fills_every_column() {
        let area = Rect::new(0, 0, 140, 40);
        let l = layout(&house(), area);
        insta::assert_snapshot!(sketch(&l, area));
        assert_sane(&l, area);
        assert_eq!(l.height, area.height);
        // Every column reaches the bottom.
        let columns: std::collections::BTreeSet<u16> = l.rooms.iter().map(|r| r.rect.x).collect();
        for x in columns {
            let bottom = l.rooms.iter().filter(|r| r.rect.x == x).map(|r| r.rect.bottom()).max().unwrap();
            assert_eq!(bottom, area.bottom(), "column at x={x} doesn't reach the bottom");
        }
        assert!(l.tiles.iter().any(|t| t.variant == Variant::Featured));
    }

    #[test]
    fn taller_terminal_means_taller_graph_tiles() {
        let short = layout(&house(), Rect::new(0, 0, 140, 30));
        let tall = layout(&house(), Rect::new(0, 0, 140, 60));
        let h = |l: &Layout, room, tile| l.tiles.iter().find(|t| t.room == room && t.tile == tile).unwrap().rect.height;
        assert!(h(&tall, 0, 1) > h(&short, 0, 1), "featured thermostat grows");
        assert!(h(&tall, 3, 0) > h(&short, 3, 0), "sensor grows");
        assert!(h(&tall, 2, 0) >= h(&short, 2, 0), "plugs grow when they're all a column has");
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
    fn tiny_terminal_scrolls() {
        let area = Rect::new(0, 0, 40, 10);
        let l = layout(&house(), area);
        assert!(l.height > area.height);
        assert!(l.tiles.iter().all(|t| t.variant == Variant::Compact));
        assert_sane(&l, area);
    }

    #[test]
    fn sane_at_many_sizes() {
        for (w, h) in [(80, 24), (120, 40), (200, 60), (38, 100), (60, 30), (240, 20)] {
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
    fn grow_shares_by_weight() {
        assert_eq!(grow(&[3, 3], &[1, 3], 10, false), vec![4, 6]);
        assert_eq!(grow(&[3, 3], &[0, 0], 10, false), vec![3, 3]);
        assert_eq!(grow(&[3, 3], &[0, 0], 10, true), vec![5, 5]);
        assert_eq!(grow(&[5], &[1], 3, false), vec![5]);
    }
}
