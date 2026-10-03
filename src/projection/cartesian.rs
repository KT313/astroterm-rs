//! Direct Cartesian camera projection of unit horizontal vectors (East, North, Up). The camera basis and scale
//! are prepared once per view, with no per-object azimuth/altitude or polar intermediate.
use super::{ProjectionKind, View, ViewCenter};
use crate::astro::Vector3;
use std::f64::consts::FRAC_PI_2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}
impl ScreenPoint {
    pub fn radius(self) -> f64 {
        self.x.hypot(self.y)
    }
    pub fn is_visible(self) -> bool {
        self.x * self.x + self.y * self.y <= 1.0 + 8.0 * f64::EPSILON
    }
    pub fn clamp_to_edge(self) -> Self {
        let radius = self.radius();
        if radius > 1.0 {
            Self {
                x: self.x / radius,
                y: self.y / radius,
            }
        } else {
            self
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CartesianCamera {
    right: Vector3,
    up: Vector3,
    forward: Vector3,
    scale: f64,
    kind: ProjectionKind,
}
impl CartesianCamera {
    pub fn new(view: &View) -> Self {
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
        Self {
            right,
            up,
            forward,
            scale,
            kind: view.projection,
        }
    }
    /// Stereographic antipodes have no finite projection. Equidistant antipodes have the legacy top-edge convention.
    pub fn project(self, direction: Vector3) -> Option<ScreenPoint> {
        let c = self.forward.dot(direction).clamp(-1.0, 1.0);
        let (x, y) = (self.right.dot(direction), self.up.dot(direction));
        match self.kind {
            ProjectionKind::Stereographic => {
                if 1.0 + c <= 0.0 || (c < 0.0 && x.hypot(y) < 1e-12) {
                    return None;
                }
                let scale = self.scale / (1.0 + c);
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
                        y: 2.0 * self.scale,
                    });
                }
                if sine == 0.0 {
                    return Some(ScreenPoint { x: 0.0, y: 0.0 });
                }
                let scale = sine.atan2(c) / FRAC_PI_2 * self.scale / sine;
                Some(ScreenPoint {
                    x: x * scale,
                    y: y * scale,
                })
            }
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
                        let old=view.project(h); let new=CartesianCamera::new(&view).project(h.to_unit_vector());
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
                let camera = CartesianCamera::new(&View {
                    center: ViewCenter::Facing {
                        azimuth: 0.7,
                        tilt: 0.4,
                    },
                    projection: kind,
                    fov_degrees: fov,
                });
                let (s, c) = (fov.to_radians() / 2.0).sin_cos();
                let edge = (camera.forward * c + camera.right * s).normalized();
                let projected = camera.project(edge).unwrap();
                assert!(
                    (projected.radius() - 1.0).abs() < 1e-10,
                    "{kind:?} {fov}: {projected:?}"
                );
                assert!(camera.project(camera.forward).unwrap().radius() < 1e-10);
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
                let camera = CartesianCamera::new(&view);
                let direction = view.center_direction().to_unit_vector();
                assert!(camera.project(direction).unwrap().radius() < 1e-12);
                if kind == ProjectionKind::Stereographic {
                    assert!(camera.project(-direction).is_none());
                } else {
                    let behind = camera.project(-direction).unwrap();
                    assert_eq!(behind, ScreenPoint { x: 0.0, y: 2.0 });
                }
            }
        }
        let camera = CartesianCamera::new(&View::default());
        assert!(camera.project(Vector3 { x: 1.0, y: 0.0, z: 0.0 }).unwrap().is_visible());
        assert!((camera.project(Vector3 { x: 1.0, y: 0.0, z: 0.0 }).unwrap().x + 1.0).abs() < 1e-12);
    }
}
