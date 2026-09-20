use super::DirtyRanges;

#[test]
fn adjacent_and_overlapping_ranges_coalesce_without_crossing_gaps() {
    let mut dirty = DirtyRanges::default();
    dirty.add(8..12);
    dirty.add(2..4);
    dirty.add(4..8);
    dirty.add(20..24);
    dirty.add(10..21);
    dirty.add(30..30);
    assert_eq!(dirty.ranges, vec![2..24]);

    dirty.add(26..28);
    assert_eq!(dirty.ranges, vec![2..24, 26..28]);
}
