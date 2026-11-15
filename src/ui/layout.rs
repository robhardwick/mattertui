//! Auto-layout: rooms and devices in declaration order → rectangles.
//!
//! Rooms are packed masonry-style into as many columns as fit, each room
//! going into the currently shortest column. If the result is taller than
//! the screen, tiles are progressively shrunk (featured → normal → compact);
//! if even that doesn't fit, the caller scrolls.

use ratatui::layout::Rect;

use crate::config::DeviceKind;

pub const MIN_ROOM_WIDTH: u16 = 34;
/// Width at which a room splits its tiles into two inner columns.
const TWO_INNER_COLUMNS_AT: u16 = 62;

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
    /// One line, no border.
    Compact,
    Normal,
    /// Big readout and chart.
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

pub fn tile_height(kind: DeviceKind, variant: Variant) -> u16 {
    match (variant, kind) {
        (Variant::Compact, _) => 1,
        (Variant::Normal | Variant::Featured, DeviceKind::Plug) => 4,
        (Variant::Normal, DeviceKind::TemperatureSensor) => 5,
        (Variant::Normal, DeviceKind::Thermostat) => 6,
        (Variant::Featured, _) => 12,
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
    let mut best = Layout::default();
    for density in [Density::Full, Density::NoFeatured, Density::Compact] {
        best = layout_at(rooms, area, density);
        if best.height <= area.height {
            break;
        }
    }
    best
}

fn layout_at(rooms: &[RoomSpec], area: Rect, density: Density) -> Layout {
    let columns = (area.width / MIN_ROOM_WIDTH).clamp(1, rooms.len() as u16);
    let mut col_heights = vec![0u16; columns as usize];
    let mut out = Layout::default();

    for (room_idx, room) in rooms.iter().enumerate() {
        // Shortest column, leftmost on ties.
        let col = (0..columns as usize)
            .min_by_key(|&c| (col_heights[c], c))
            .unwrap_or(0);
        let (x, width) = column_span(area, columns, col as u16);
        let y = area.y + col_heights[col];

        let inner = Rect::new(x + 1, y + 1, width.saturating_sub(2), 0);
        let (tiles, inner_height) = place_tiles(room_idx, room, inner, density);
        let rect = Rect::new(x, y, width, inner_height + 2);

        col_heights[col] += rect.height;
        out.rooms.push(PlacedRoom {
            room: room_idx,
            rect,
        });
        out.tiles.extend(tiles);
    }
    out.height = col_heights.into_iter().max().unwrap_or(0);
    out
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

/// Place a room's tiles inside `inner` (whose height is ignored). Returns
/// the tiles and the height they need.
fn place_tiles(
    room_idx: usize,
    room: &RoomSpec,
    inner: Rect,
    density: Density,
) -> (Vec<PlacedTile>, u16) {
    let inner_cols: u16 = if inner.width >= TWO_INNER_COLUMNS_AT { 2 } else { 1 };
    let mut heights = vec![0u16; inner_cols as usize];
    let mut tiles = Vec::new();

    for (tile_idx, spec) in room.tiles.iter().enumerate() {
        let v = variant(*spec, density);
        let h = tile_height(spec.kind, v);
        if v == Variant::Featured && inner_cols > 1 {
            // Featured tiles span the room: level the columns first.
            let top = heights.iter().copied().max().unwrap_or(0);
            tiles.push(PlacedTile {
                room: room_idx,
                tile: tile_idx,
                rect: Rect::new(inner.x, inner.y + top, inner.width, h),
                variant: v,
            });
            heights.iter_mut().for_each(|c| *c = top + h);
        } else {
            let col = (0..inner_cols as usize)
                .min_by_key(|&c| (heights[c], c))
                .unwrap_or(0);
            let (x, w) = column_span(inner, inner_cols, col as u16);
            tiles.push(PlacedTile {
                room: room_idx,
                tile: tile_idx,
                rect: Rect::new(x, inner.y + heights[col], w, h),
                variant: v,
            });
            heights[col] += h;
        }
    }
    let height = heights.into_iter().max().unwrap_or(0).max(1);
    (tiles, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use DeviceKind::*;

    fn room(kinds: &[(DeviceKind, bool)]) -> RoomSpec {
        RoomSpec {
            tiles: kinds
                .iter()
                .map(|&(kind, featured)| TileSpec { kind, featured })
                .collect(),
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

    #[test]
    fn wide_terminal_uses_columns_and_featured_tiles() {
        let area = Rect::new(0, 0, 140, 40);
        let l = layout(&house(), area);
        insta::assert_snapshot!(sketch(&l, area));
        assert!(l.height <= area.height);
        assert!(l.tiles.iter().any(|t| t.variant == Variant::Featured));
    }

    #[test]
    fn standard_terminal_fits_at_full_density() {
        let area = Rect::new(0, 0, 80, 24);
        let l = layout(&house(), area);
        insta::assert_snapshot!(sketch(&l, area));
        assert!(l.height <= area.height);
    }

    #[test]
    fn tiny_terminal_scrolls() {
        let area = Rect::new(0, 0, 40, 10);
        let l = layout(&house(), area);
        assert!(l.height > area.height);
        assert!(l.tiles.iter().all(|t| t.variant == Variant::Compact));
    }

    #[test]
    fn rooms_and_tiles_never_overlap() {
        for (w, h) in [(80, 24), (120, 40), (200, 60), (34, 100)] {
            let l = layout(&house(), Rect::new(0, 0, w, h));
            for (i, a) in l.rooms.iter().enumerate() {
                for b in &l.rooms[i + 1..] {
                    assert!(!a.rect.intersects(b.rect), "{w}x{h}: {a:?} {b:?}");
                }
            }
            for (i, a) in l.tiles.iter().enumerate() {
                for b in &l.tiles[i + 1..] {
                    assert!(!a.rect.intersects(b.rect), "{w}x{h}: {a:?} {b:?}");
                }
                let room = l.rooms[a.room].rect;
                assert!(room.contains(a.rect.as_position()), "tile outside its room");
                assert!(a.rect.right() < room.right() && a.rect.bottom() < room.bottom());
            }
        }
    }

    #[test]
    fn respects_area_offset() {
        let l = layout(&house(), Rect::new(5, 3, 120, 40));
        assert!(l.rooms.iter().all(|r| r.rect.x >= 5 && r.rect.y >= 3));
    }
}
