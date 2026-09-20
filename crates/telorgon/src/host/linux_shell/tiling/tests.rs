use super::*;
#[test]
fn preview_padding_insets_outer_edges_without_opening_tile_seams() {
    let area = RectI {
        x: 17,
        y: 41,
        width: 1001,
        height: 703,
    };
    for splits in [[0.5, 0.5, 0.5], [0.37, 0.61, 0.42]] {
        let padding = crate::authoring::compose::Insets::new(7.0, 11.0, 13.0, 17.0);
        for target in [
            TileTarget::Left,
            TileTarget::Right,
            TileTarget::TopLeft,
            TileTarget::TopRight,
            TileTarget::BottomLeft,
            TileTarget::BottomRight,
        ] {
            let raw = tile_rect(area, splits, target);
            let padded = preview_rect(area, splits, target, padding);
            assert_eq!(
                preview_rect(area, splits, target, crate::authoring::compose::Insets::ZERO),
                raw
            );
            assert_eq!(padded.x, raw.x + if target.left() { 17 } else { 0 });
            assert_eq!(
                padded.right(),
                raw.right() - if target.left() { 0 } else { 11 }
            );
            assert_eq!(
                padded.y,
                raw.y + if target.row() != Some(true) { 7 } else { 0 }
            );
            assert_eq!(
                padded.bottom(),
                raw.bottom() - if target.row() != Some(false) { 13 } else { 0 }
            );
        }
        let top = preview_rect(area, splits, TileTarget::TopLeft, padding);
        let bottom = preview_rect(area, splits, TileTarget::BottomLeft, padding);
        let right = preview_rect(area, splits, TileTarget::Right, padding);
        assert_eq!(top.bottom(), bottom.y);
        assert_eq!(top.right(), right.x);
        assert_eq!(bottom.right(), right.x);
    }
}

#[test]
fn preview_padding_rounds_and_clamps_to_nonempty_bounds() {
    let area = RectI {
        x: 0,
        y: 30,
        width: 101,
        height: 71,
    };
    let splits = [0.5; 3];
    let raw = tile_rect(area, splits, TileTarget::Left);
    let rounded = preview_rect(
        area,
        splits,
        TileTarget::Left,
        crate::authoring::compose::Insets::all(2.5),
    );
    assert_eq!(rounded.x, raw.x + 3);
    assert_eq!(rounded.y, raw.y + 3);
    assert_eq!(rounded.right(), raw.right());
    for target in [
        TileTarget::Left,
        TileTarget::Right,
        TileTarget::TopLeft,
        TileTarget::TopRight,
        TileTarget::BottomLeft,
        TileTarget::BottomRight,
    ] {
        let raw = tile_rect(area, splits, target);
        let padded = preview_rect(area, splits, target, crate::authoring::compose::Insets::all(f32::MAX));
        assert_eq!(padded.width, 1);
        assert_eq!(padded.height, 1);
        assert!(padded.x >= raw.x && padded.y >= raw.y);
        assert!(padded.right() <= raw.right() && padded.bottom() <= raw.bottom());
        if target.left() {
            assert_eq!(padded.right(), raw.right());
        } else {
            assert_eq!(padded.x, raw.x);
        }
    }
}

#[test]
fn odd_work_area_partitions_without_gaps() {
    let area = RectI {
        x: 17,
        y: 41,
        width: 1001,
        height: 703,
    };
    let splits = [0.37, 0.61, 0.42];
    let a = tile_rect(area, splits, TileTarget::TopLeft);
    let b = tile_rect(area, splits, TileTarget::BottomLeft);
    let c = tile_rect(area, splits, TileTarget::Right);
    assert_eq!(a.bottom(), b.y);
    assert_eq!(a.right(), c.x);
    assert_eq!(b.bottom(), area.bottom());
    assert_eq!(c.right(), area.right());
    assert_eq!(
        a.width * a.height + b.width * b.height + c.width * c.height,
        area.width * area.height
    );
}
