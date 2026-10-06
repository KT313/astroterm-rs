//! Direct Cartesian camera projection of unit horizontal vectors (East, North, Up). The camera basis and scale
//! are prepared once per view, with no per-object azimuth/altitude or polar intermediate.
use crate::model::{ProjectionKind, View, ViewCenter};
use crate::astro::Vector3;
use std::f64::consts::FRAC_PI_2;

use crate::model::{CartesianCamera, ScreenPoint};

/// Prepare the Cartesian camera basis and projection scale once for a view.
pub fn prepare_camera(view: &View) -> CartesianCamera {
    let (right, up, forward) = match view.center {
        ViewCenter::Zenith => (
            Vector3 {
                x: -1.0,
                y: 0.0,
                z: 0.0,
            },
            Vector3 { x: 0.0, y: 1.0, z: 0.0 },
            Vector3 { x: 0.0, y: 0.0, z: 1.0 },
        ),
        ViewCenter::Facing { azimuth, tilt } => {
            let (sa, ca) = azimuth.sin_cos();
            let (st, ct) = tilt.sin_cos();
            (
                Vector3 { x: ca, y: -sa, z: 0.0 },
                Vector3 {
                    x: -st * sa,
                    y: -st * ca,
                    z: ct,
                },
                Vector3 {
                    x: ct * sa,
                    y: ct * ca,
                    z: st,
                },
            )
        }
    };
    let scale = match view.projection {
        ProjectionKind::Stereographic if view.fov_degrees == 180.0 => 1.0,
        ProjectionKind::Stereographic => 1.0 / (view.fov_degrees.to_radians() / 4.0).tan(),
        ProjectionKind::Equidistant => 180.0 / view.fov_degrees,
    };
    CartesianCamera {
        right,
        up,
        forward,
        scale,
        kind: view.projection,
    }
}

/// Project a unit horizontal vector directly, preserving singular and antipodal handling.
pub fn project_camera(camera: CartesianCamera, direction: Vector3) -> Option<ScreenPoint> {
    let c = camera.forward.dot(direction).clamp(-1.0, 1.0);
    let (x, y) = (camera.right.dot(direction), camera.up.dot(direction));
    match camera.kind {
        ProjectionKind::Stereographic => {
            if 1.0 + c <= 0.0 || (c < 0.0 && x.hypot(y) < 1e-12) {
                return None;
            }
            let scale = camera.scale / (1.0 + c);
            Some(ScreenPoint {
                x: x * scale,
                y: y * scale,
            })
        }
        ProjectionKind::Equidistant => {
            let sine = x.hypot(y);
            if sine < 1e-12 && c < 0.0 {
                return Some(ScreenPoint {
                    x: 0.0,
                    y: 2.0 * camera.scale,
                });
            }
            if sine == 0.0 {
                return Some(ScreenPoint { x: 0.0, y: 0.0 });
            }
            let scale = sine.atan2(c) / FRAC_PI_2 * camera.scale / sine;
            Some(ScreenPoint {
                x: x * scale,
                y: y * scale,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astro::Horizontal;
    use proptest::prelude::*;
    use std::f64::consts::PI;
    proptest! {
        #[test]
        fn screen_coordinates_match_legacy_to_one_hundredth_of_a_cell(
            az in -PI..PI, alt in -FRAC_PI_2..FRAC_PI_2, facing in -PI..PI, tilt in -FRAC_PI_2..FRAC_PI_2
        ) {
            for kind in [ProjectionKind::Stereographic,ProjectionKind::Equidistant] {
                for fov in [1.0,180.0,359.0,360.0] {
                    if kind==ProjectionKind::Stereographic && fov==360.0 {continue;}
                    for center in [ViewCenter::Zenith,ViewCenter::Facing{azimuth:facing,tilt}] {
                        let view=View{center,projection:kind,fov_degrees:fov};
                        let h=Horizontal{azimuth:az,altitude:alt};
                        let old=crate::projection::project_horizontal(&view, h); let new=crate::projection::project_camera(crate::projection::prepare_camera(&view), h.to_unit_vector());
                        if old.radius<=1.0 {
                            let new=new.unwrap(); let (x,y)=old.to_cartesian();
                            prop_assert!((new.x-x).abs()*1000.0<0.01);
                            prop_assert!((new.y-y).abs()*500.0<0.01);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn field_edges_include_narrow_wide_and_full_views() {
        for kind in [ProjectionKind::Stereographic, ProjectionKind::Equidistant] {
            for fov in [1.0, 180.0, 359.0, 360.0] {
                if kind == ProjectionKind::Stereographic && fov == 360.0 {
                    continue;
                }
                let camera = crate::projection::prepare_camera(&View {
                    center: ViewCenter::Facing {
                        azimuth: 0.7,
                        tilt: 0.4,
                    },
                    projection: kind,
                    fov_degrees: fov,
                });
                let (s, c) = (fov.to_radians() / 2.0).sin_cos();
                let edge = (camera.forward * c + camera.right * s).normalized();
                let projected = crate::projection::project_camera(camera, edge).unwrap();
                assert!(
                    (projected.radius() - 1.0).abs() < 1e-10,
                    "{kind:?} {fov}: {projected:?}"
                );
                assert!(crate::projection::project_camera(camera, camera.forward).unwrap().radius() < 1e-10);
            }
        }
    }

    #[test]
    fn center_horizon_and_antipodes_have_defined_limits() {
        for center in [
            ViewCenter::Zenith,
            ViewCenter::Facing {
                azimuth: 0.3,
                tilt: 0.4,
            },
        ] {
            for kind in [ProjectionKind::Stereographic, ProjectionKind::Equidistant] {
                let view = View {
                    center,
                    projection: kind,
                    fov_degrees: 180.0,
                };
                let camera = crate::projection::prepare_camera(&view);
                let direction = view.center_direction().to_unit_vector();
                assert!(crate::projection::project_camera(camera, direction).unwrap().radius() < 1e-12);
                if kind == ProjectionKind::Stereographic {
                    assert!(crate::projection::project_camera(camera, -direction).is_none());
                } else {
                    let behind = crate::projection::project_camera(camera, -direction).unwrap();
                    assert_eq!(behind, ScreenPoint { x: 0.0, y: 2.0 });
                }
            }
        }
        let camera = crate::projection::prepare_camera(&View::default());
        assert!(crate::projection::project_camera(camera, Vector3 { x: 1.0, y: 0.0, z: 0.0 }).unwrap().is_visible());
        assert!((crate::projection::project_camera(camera, Vector3 { x: 1.0, y: 0.0, z: 0.0 }).unwrap().x + 1.0).abs() < 1e-12);
    }
}
