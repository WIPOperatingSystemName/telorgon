use super::*;
use crate::host::linux_shell::motion::SnapshotInput;

#[test]
fn glass_recipe_preserves_window_alignment_through_padded_interrupted_mix() {
    let mut recipes = BTreeMap::new();
    extract_recipe(&capture(1), &styles(), &mut recipes);
    let padded = RectF {
        x: 0.1,
        y: 0.2,
        width: 0.8,
        height: 0.6,
    };
    let extent = SizeI {
        width: 500,
        height: 500,
    };
    extract_recipe(
        &SnapshotCommand {
            id: 2,
            extent,
            content: SnapshotContent::Mix(vec![SnapshotInput {
                id: 1,
                weight: 0.5,
                target: padded,
            }]),
        },
        &BTreeMap::new(),
        &mut recipes,
    );
    let output = ShellPlacement {
        target: full_rect(extent),
        ..veil()
    };
    let placement = lens_placement(&recipes[&2][0], output);
    assert_eq!(
        placement.target,
        RectI {
            x: 50,
            y: 100,
            width: 400,
            height: 300
        }
    );
    // Return to the unpadded canvas without baking intermediate rounded coordinates.
    let unit = SnapshotInput::from((0, 1.0)).target;
    extract_recipe(
        &SnapshotCommand {
            id: 3,
            extent: capture(1).extent,
            content: SnapshotContent::Mix(vec![SnapshotInput::aligned(2, 0.5, unit, padded)]),
        },
        &BTreeMap::new(),
        &mut recipes,
    );
    assert_eq!(
        lens_placement(&recipes[&3][0], veil()).target,
        veil().target
    );
    assert_eq!(recipes[&3][0].weight, 0.25);
}
