use super::*;
use parquet::{record::Field, schema::parser::parse_message_type};

fn schema() -> Type {
    parse_message_type("message gaia {
        OPTIONAL INT64 source_id; OPTIONAL BYTE_ARRAY designation (UTF8);
        OPTIONAL DOUBLE ra; OPTIONAL DOUBLE dec; OPTIONAL DOUBLE ref_epoch;
        OPTIONAL DOUBLE pmra; OPTIONAL DOUBLE pmdec; OPTIONAL DOUBLE parallax;
        OPTIONAL FLOAT parallax_over_error; OPTIONAL FLOAT radial_velocity;
        OPTIONAL FLOAT phot_g_mean_mag; OPTIONAL FLOAT phot_bp_mean_mag; OPTIONAL FLOAT phot_rp_mean_mag;
    }").unwrap()
}

/// Retain the former record-reader path only as a test oracle for the new column decoder.
fn read_reference(bytes: Bytes) -> Vec<MastRow> {
    let reader = SerializedFileReader::new(bytes).unwrap();
    let projection = projected_schema(reader.metadata()).unwrap();
    reader.get_row_group(0).unwrap().get_row_iter(Some(projection)).unwrap().map(|record| {
        let mut row = MastRow { source_id: 0, designation: None, values: [None; 11] };
        for (name, field) in record.unwrap().into_columns() {
            let index = COLUMNS.iter().position(|column| *column == name).unwrap();
            match (index, field) {
                (0, Field::Long(id)) => row.source_id = id as u64,
                (1, Field::Str(label)) => row.designation = Some(label),
                (_, Field::Null) => {},
                (index, Field::Double(value)) => row.values[index - 2] = Some(value.to_bits()),
                (index, Field::Float(value)) => row.values[index - 2] = Some(f64::from(value).to_bits()),
                _ => panic!("unexpected test schema"),
            }
        }
        row
    }).collect()
}

#[test]
fn column_decoder_matches_record_decoder_across_batch_boundaries() {
    let rows: Vec<_> = (0..MAST_DECODE_BATCH_ROWS + 3).map(|i| {
        let mut row = MastRow { source_id: (1 << 55) + i as u64, designation: (i % 5 != 0).then(|| format!("source α {i}")), values: [None; 11] };
        for (column, value) in row.values.iter_mut().enumerate() {
            if (i + column) % 3 != 0 { *value = Some((i as f64 * 0.123 + column as f64).to_bits()); }
        }
        row.values[0] = Some(if i % 2 == 0 { (-0.0f64).to_bits() } else { 0x7ff8_0000_0000_0042 });
        row
    }).collect();
    let bytes = write_block(&rows, schema()).unwrap();
    assert_eq!(read_block(bytes.clone()).unwrap(), read_reference(Bytes::from(bytes)));
}

#[test]
fn column_decoder_rejects_misleading_logical_id_and_label_types() {
    let row = MastRow { source_id: 10, designation: Some("Gaia DR3 10".into()), values: [Some(0f64.to_bits()); 11] };
    for (name, converted) in [("source_id", ConvertedType::UINT_64), ("designation", ConvertedType::NONE)] {
        let fields = schema().get_fields().iter().map(|field| {
            if field.name() != name { return Arc::clone(field); }
            Arc::new(Type::primitive_type_builder(name, field.get_physical_type()).with_repetition(parquet::basic::Repetition::OPTIONAL)
                .with_converted_type(converted).build().unwrap())
        }).collect();
        let schema = Type::group_type_builder("gaia").with_fields(fields).build().unwrap();
        let bytes = write_block(std::slice::from_ref(&row), schema).unwrap();
        assert!(read_block(bytes).is_err());
    }
}

#[test]
#[ignore = "local sample equivalence; requires ASTROTERM_MAST_SAMPLE, no network"]
fn mast_compare_real_column_and_record_decoders() {
    let path = std::env::var_os("ASTROTERM_MAST_SAMPLE").expect("set a local normalized Parquet block");
    let bytes = std::fs::read(path).unwrap();
    let actual = read_block(bytes.clone()).unwrap();
    let expected = read_reference(Bytes::from(bytes));
    assert_eq!(actual, expected);
    eprintln!("{} real rows match the previous decoder bit-for-bit", actual.len());
}
