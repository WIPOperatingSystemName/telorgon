use super::*;
#[test]
fn validation_borrows_unchanged_storage_and_keeps_patches_transactional() {
    let retained = vec![10, 20, 30];
    let unchanged = patched_view(&retained, &[], 3);
    assert_eq!(unchanged.as_ptr(), retained.as_ptr());
    let shrunk = patched_view(&retained, &[], 2);
    assert_eq!(shrunk.as_ptr(), retained.as_ptr());
    assert_eq!(&*shrunk, &[10, 20]);
    let patched = patched_view(
        &retained,
        &[RangePatch {
            start: 1,
            values: vec![99, 88, 77].into(),
        }],
        4,
    );
    assert_eq!(&*patched, &[10, 99, 88, 77]);
    assert_eq!(
        retained,
        [10, 20, 30],
        "validation must not mutate the committed scene"
    );
}
