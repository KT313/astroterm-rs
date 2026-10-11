use super::*;
use parquet::{record::Field, schema::parser::parse_message_type};

fn schema() -> Type {
    parse_message_type("message gaia {
        OPTIONAL INT64 source_id;
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
        let mut row = MastRow { source_id: 0, values: [None; 11] };
        for (name, field) in record.unwrap().into_columns() {
            let index = COLUMNS.iter().position(|column| *column == name).unwrap();
            match (index, field) {
                (0, Field::Long(id)) => row.source_id = id as u64,
                (_, Field::Null) => {},
                (index, Field::Double(value)) => row.values[index - 1] = Some(value.to_bits()),
                (index, Field::Float(value)) => row.values[index - 1] = Some(f64::from(value).to_bits()),
                _ => panic!("unexpected test schema"),
            }
        }
        row
    }).collect()
}

#[test]
fn column_decoder_matches_record_decoder_across_batch_boundaries() {
    let rows: Vec<_> = (0..MAST_DECODE_BATCH_ROWS + 3).map(|i| {
        let mut row = MastRow { source_id: (1 << 55) + i as u64, values: [None; 11] };
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
fn column_decoder_rejects_misleading_logical_id_types() {
    let row = MastRow { source_id: 10, values: [Some(0f64.to_bits()); 11] };
    for (name, converted) in [("source_id", ConvertedType::UINT_64), ("source_id", ConvertedType::TIMESTAMP_MILLIS)] {
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

#[test]
fn legacy_designation_column_is_ignored_and_new_normalization_is_deterministic() {
    use parquet::data_type::{ByteArray, ByteArrayType, DoubleType};
    let mut fields = schema().get_fields().to_vec();
    fields.push(Arc::new(Type::primitive_type_builder("designation", Physical::BYTE_ARRAY)
        .with_repetition(parquet::basic::Repetition::OPTIONAL).with_converted_type(ConvertedType::UTF8).build().unwrap()));
    let legacy = Type::group_type_builder("legacy").with_fields(fields).build().unwrap();
    let mut writer = SerializedFileWriter::new(Vec::new(), Arc::new(legacy), Default::default()).unwrap();
    let mut group = writer.next_row_group().unwrap();
    let input = MastRow { source_id: (1 << 55) + 17, values: [Some(1f64.to_bits()); 11] };
    for index in 0..13 {
        let mut column = group.next_column().unwrap().unwrap();
        match index {
            0 => { column.typed::<Int64Type>().write_batch(&[input.source_id as i64], Some(&[1]), None).unwrap(); },
            12 => { column.typed::<ByteArrayType>().write_batch(&[ByteArray::from("ignored legacy text")], Some(&[1]), None).unwrap(); },
            _ => match column.untyped() {
                parquet::column::writer::ColumnWriter::FloatColumnWriter(writer) => { writer.write_batch(&[1.0], Some(&[1]), None).unwrap(); },
                _ => { column.typed::<DoubleType>().write_batch(&[1.0], Some(&[1]), None).unwrap(); },
            },
        }
        column.close().unwrap();
    }
    group.close().unwrap();
    let decoded = read_block(writer.into_inner().unwrap()).unwrap();
    assert_eq!(decoded, [input]);
    let a = write_block(&decoded, schema()).unwrap();
    let b = write_block(&decoded, schema()).unwrap();
    assert_eq!(a, b);
    let reader = SerializedFileReader::new(Bytes::from(a)).unwrap();
    assert_eq!(reader.metadata().file_metadata().schema().get_fields().len(), 12);
}
