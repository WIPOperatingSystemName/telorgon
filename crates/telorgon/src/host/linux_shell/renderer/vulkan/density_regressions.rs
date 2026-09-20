use super::*;
#[test]
fn x11_coordinate_density_is_removed_before_raster_scaling() {
    for scale in [1.0_f32, 1.5, 2.0, 3.0] {
        let density = scale.ceil();
        let extent = SizeI {
            width: (300.0 * density) as i32,
            height: (200.0 * density) as i32,
        };
        assert_eq!(
            materialization_extent(
                extent,
                super::super::super::geometry::surface_raster_scale(
                    crate::platform::contracts::ScaleFactor::new(scale).unwrap(),
                    density as i32
                )
                .get()
            ),
            SizeI {
                width: (300.0 * scale) as i32,
                height: (200.0 * scale) as i32,
            }
        );
    }
    assert_eq!(
        materialization_extent(
            SizeI {
                width: 901,
                height: 601
            },
            1.0
        ),
        SizeI {
            width: 901,
            height: 601
        }
    );
}
