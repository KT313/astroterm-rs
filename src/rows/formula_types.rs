//! Diagnostic row formatting for physical types; formulas do not depend on the table logger.
use crate::formulas::{Vector3, Matrix3, Observer, MoonPhase, BodyState, StellarMotion, StellarSample};
use super::row_columns;

row_columns!(Vector3 { x, y, z });
row_columns!(BodyState { position, velocity });
row_columns!(StellarSample { direction, magnitude, used_singular_fallback });
crate::rows::debug_preview!(Matrix3, Observer, MoonPhase, StellarMotion);
