//! Analytic derivatives of the same direction corrections used by the observer. A corner carries its value,
//! changes along tile axes u and v, and the mixed change uv; no neighboring sky positions are sampled.
use crate::astro::{Horizontal, LIGHT_SPEED_AU_DAY, Matrix3, Vector3, apply_refraction, refract_direction};
use super::coords::refraction_derivatives;

#[derive(Clone, Copy, Debug)]
pub struct DirectionDerivatives {
    pub value: Vector3,
    pub along_u: Vector3,
    pub along_v: Vector3,
    pub mixed: Vector3,
}

impl DirectionDerivatives {
    /// Propagate first and mixed derivatives through normalization of a nonzero vector.
    pub fn normalize(self) -> Self {
        // Differentiate the length before applying the quotient rule to each vector component.
        let length = self.value.length();
        let value = self.value.normalized();
        let length_u = value.dot(self.along_u);
        let length_v = value.dot(self.along_v);
        let along_u = (self.along_u - value * length_u) * (1.0 / length);
        let along_v = (self.along_v - value * length_v) * (1.0 / length);
        let length_mixed = along_v.dot(self.along_u) + value.dot(self.mixed);
        let mixed = (self.mixed - along_u * length_v - along_v * length_u - value * length_mixed) * (1.0 / length);
        Self { value, along_u, along_v, mixed }
    }
}

/// Differentiate normalization, the existing velocity correction, horizon rotation and optional refraction.
pub fn transform_direction_derivatives(
    direction: DirectionDerivatives, velocity: Vector3, rotation: Matrix3, atmosphere: bool,
) -> Option<DirectionDerivatives> {
    // Aberration adds a constant observer velocity between two normalizations.
    let mut apparent = direction.normalize();
    apparent.value = apparent.value + velocity * (1.0 / LIGHT_SPEED_AU_DAY);
    apparent = apparent.normalize();
    let horizon = DirectionDerivatives {
        value: rotation.apply(apparent.value),
        along_u: rotation.apply(apparent.along_u),
        along_v: rotation.apply(apparent.along_v),
        mixed: rotation.apply(apparent.mixed),
    };
    if !atmosphere { return Some(horizon); }

    // Recover altitude derivatives. At an exact pole the current refraction convention has no unique azimuth.
    let DirectionDerivatives { value, along_u, along_v, mixed } = horizon;
    let horizontal = value.x.hypot(value.y);
    if horizontal < 1e-12 { return None; }
    let horizontal_u = (value.x * along_u.x + value.y * along_u.y) / horizontal;
    let horizontal_v = (value.x * along_v.x + value.y * along_v.y) / horizontal;
    let horizontal_mixed = (along_v.x * along_u.x + value.x * mixed.x + along_v.y * along_u.y
        + value.y * mixed.y - horizontal_u * horizontal_v) / horizontal;
    let norm_squared = value.dot(value);
    let altitude = value.z.atan2(horizontal);
    let altitude_u = (horizontal * along_u.z - value.z * horizontal_u) / norm_squared;
    let altitude_v = (horizontal * along_v.z - value.z * horizontal_v) / norm_squared;
    let altitude_mixed = (horizontal_v * along_u.z + horizontal * mixed.z - along_v.z * horizontal_u
        - value.z * horizontal_mixed - altitude_u * 2.0 * value.dot(along_v)) / norm_squared;

    // Apply the chain rule to the active refraction branch, including its second derivative.
    let corrected = apply_refraction(Horizontal { azimuth: 0.0, altitude }).altitude;
    let (slope, curvature) = refraction_derivatives(altitude);
    let corrected_u = slope * altitude_u;
    let corrected_v = slope * altitude_v;
    let corrected_mixed = curvature * altitude_u * altitude_v + slope * altitude_mixed;
    let (sine, cosine) = corrected.sin_cos();
    let cosine_u = -sine * corrected_u;
    let cosine_v = -sine * corrected_v;
    let cosine_mixed = -cosine * corrected_u * corrected_v - sine * corrected_mixed;

    // Differentiate the horizontal rescale and the corrected vertical coordinate.
    let scale = cosine / horizontal;
    let scale_u = (cosine_u - scale * horizontal_u) / horizontal;
    let scale_v = (cosine_v - scale * horizontal_v) / horizontal;
    let scale_mixed = (cosine_mixed - scale_u * horizontal_v - scale_v * horizontal_u - scale * horizontal_mixed) / horizontal;
    Some(DirectionDerivatives {
        value: refract_direction(value),
        along_u: Vector3 {
            x: along_u.x * scale + value.x * scale_u,
            y: along_u.y * scale + value.y * scale_u,
            z: cosine * corrected_u,
        },
        along_v: Vector3 {
            x: along_v.x * scale + value.x * scale_v,
            y: along_v.y * scale + value.y * scale_v,
            z: cosine * corrected_v,
        },
        mixed: Vector3 {
            x: mixed.x * scale + along_u.x * scale_v + along_v.x * scale_u + value.x * scale_mixed,
            y: mixed.y * scale + along_u.y * scale_v + along_v.y * scale_u + value.y * scale_mixed,
            z: -sine * corrected_u * corrected_v + cosine * corrected_mixed,
        },
    })
}
