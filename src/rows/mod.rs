//! Column names and types of the records that `state::log_data` prints as table rows.
//!
//! A row type declares its columns once, next to its definition, with `row_columns!`; the field list is
//! compiler-checked because the macro destructures `Self { a, b, c }`. Primitives, arrays, tuples and `Option`s
//! are one unnamed column each, or one per tuple element. Types are named by `std::any::type_name` and shortened
//! for display by `short_type_name`.
use std::fmt::Debug;

/// One column of a table row: the field name (empty for a plain value) and its Rust type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: &'static str,
    pub dtype: &'static str,
}

/// A type that can be one row of a logged table.
pub trait Row: Debug {
    /// Names and types in field order.
    fn columns() -> Vec<Column>;
    /// One text per column, in the same order; the default is the whole row's Debug text.
    fn cells(&self) -> Vec<String> { vec![format!("{self:?}")] }
}

/// The type name of a field without needing a value: the closure is never called.
pub fn field_type<R, F>(_: fn(&R) -> &F) -> &'static str {
    std::any::type_name::<F>()
}

/// Type name of a plain value, as one unnamed column.
pub fn plain_column<T: ?Sized>() -> Vec<Column> {
    vec![Column { name: "", dtype: std::any::type_name::<T>() }]
}

/// `alloc::vec::Vec<astroterm::astro::Vector3>` → `Vec<Vector3>`: drop every `path::` prefix, also inside generics.
pub fn short_type_name(full: &str) -> String {
    let mut text = full.to_string();
    while let Some(separator) = text.find("::") {
        let start = text[..separator].rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
        text.replace_range(start..separator + 2, "");
    }
    text
}

/// Declare the columns of a struct row next to its definition. `..` allows fields that are not listed (used for
/// feature-gated fields). Generic rows take their parameters as `Name<T>`.
macro_rules! row_columns {
    (@impl $type:ident $(<$($generic:ident),+>)? { $($field:ident),+ } [$($rest:tt)*]) => {
        impl$(<$($generic: ::std::fmt::Debug),+>)? $crate::rows::Row for $type$(<$($generic),+>)? {
            fn columns() -> Vec<$crate::rows::Column> {
                vec![$($crate::rows::Column { name: stringify!($field), dtype: $crate::rows::field_type(|row: &Self| &row.$field) }),+]
            }
            fn cells(&self) -> Vec<String> {
                let Self { $($field),+ $($rest)* } = self; // every field must be listed above, or the build fails here
                vec![$(format!("{:?}", $field)),+]
            }
        }
    };
    ($type:ident $(<$($generic:ident),+>)? { $($field:ident),+ , .. }) => {
        $crate::rows::row_columns!(@impl $type $(<$($generic),+>)? { $($field),+ } [, ..]);
    };
    ($type:ident $(<$($generic:ident),+>)? { $($field:ident),+ }) => {
        $crate::rows::row_columns!(@impl $type $(<$($generic),+>)? { $($field),+ } []);
    };
}
pub(crate) use row_columns;

/// Types shown as one unnamed column with their Debug text (primitives, newtypes, enums).
macro_rules! plain_rows {
    ($($type:ty),+ $(,)?) => { $(
        impl $crate::rows::Row for $type {
            fn columns() -> Vec<$crate::rows::Column> { $crate::rows::plain_column::<$type>() }
        }
    )+ };
}
pub(crate) use plain_rows;

plain_rows!(bool, char, u8, u16, u32, u64, usize, i32, i64, f32, f64, &'static str, String);

impl<T: Debug, const N: usize> Row for [T; N] {
    fn columns() -> Vec<Column> { plain_column::<Self>() }
}
impl<T: Debug> Row for Option<T> {
    fn columns() -> Vec<Column> { plain_column::<Self>() }
}

/// Tuples: one unnamed column per element.
macro_rules! tuple_rows {
    ($( ($($element:ident),+) ),+ $(,)?) => { $(
        #[allow(non_snake_case)]
        impl<$($element: Debug),+> Row for ($($element,)+) {
            fn columns() -> Vec<Column> { vec![$(Column { name: "", dtype: std::any::type_name::<$element>() }),+] }
            fn cells(&self) -> Vec<String> {
                let ($($element,)+) = self;
                vec![$(format!("{:?}", $element)),+]
            }
        }
    )+ };
}
tuple_rows!((A, B), (A, B, C), (A, B, C, D));

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Fixture { index: usize, label: &'static str, position: Option<(f64, f64)> }
    row_columns!(Fixture { index, label, position });

    #[derive(Debug)]
    struct Generic<T> { epoch: f64, value: T }
    row_columns!(Generic<T> { epoch, value });

    #[test]
    fn struct_rows_declare_names_types_and_cells() {
        let columns = Fixture::columns();
        assert_eq!(columns.iter().map(|c| c.name).collect::<Vec<_>>(), ["index", "label", "position"]);
        assert_eq!(columns.iter().map(|c| short_type_name(c.dtype)).collect::<Vec<_>>(), ["usize", "&str", "Option<(f64, f64)>"]);
        let row = Fixture { index: 3, label: "a", position: Some((1.0, 2.5)) };
        assert_eq!(row.cells(), ["3", "\"a\"", "Some((1.0, 2.5))"]);
    }

    #[test]
    fn generic_rows_resolve_the_parameter_per_use() {
        let columns = Generic::<[u8; 3]>::columns();
        assert_eq!(short_type_name(columns[1].dtype), "[u8; 3]");
        assert_eq!(Generic { epoch: 1.0, value: [1_u8, 2, 3] }.cells(), ["1.0", "[1, 2, 3]"]);
    }

    #[test]
    fn plain_and_tuple_rows() {
        assert_eq!(f32::columns(), [Column { name: "", dtype: "f32" }]);
        assert_eq!(<[u8; 16]>::columns()[0].dtype, "[u8; 16]");
        assert_eq!(2.5_f32.cells(), ["2.5"]);
        let columns = <(usize, (i32, i32))>::columns();
        assert_eq!(columns.iter().map(|c| c.dtype).collect::<Vec<_>>(), ["usize", "(i32, i32)"]);
        assert_eq!((7_usize, (1_i32, 2_i32)).cells(), ["7", "(1, 2)"]);
    }

    #[test]
    fn short_names_drop_module_paths_inside_generics() {
        assert_eq!(short_type_name("alloc::vec::Vec<astroterm::astro::Vector3>"), "Vec<Vector3>");
        assert_eq!(short_type_name("core::option::Option<(usize, f64)>"), "Option<(usize, f64)>");
        assert_eq!(short_type_name("&str"), "&str");
        assert_eq!(short_type_name("[u8; 16]"), "[u8; 16]");
        assert_eq!(short_type_name("(a::B, c::d::E<f::G>)"), "(B, E<G>)");
    }
}
