//! Project only the required scalar fields. Missing ranges are errors, never implicit zero-filled data.
use super::{COLUMNS, storage::invalid};
use crate::constants::{MAST_DECODED_MAX_BYTES, MAST_DECODE_BATCH_ROWS, MAST_BLOCK_MAX_BYTES};
use bytes::Bytes;
use parquet::{basic::{Type as Physical, Compression}, data_type::{Int64Type, ByteArrayType, ByteArray},
    file::{reader::{ChunkReader, Length, FileReader}, serialized_reader::SerializedFileReader, metadata::{ParquetMetaData, ParquetMetaDataReader}, writer::SerializedFileWriter, properties::WriterProperties},
    schema::types::Type, record::Field};
use serde::{Serialize, Deserialize};
use std::{io::{self, Cursor}, sync::Arc};

/// Bit-preserving numeric inputs make duplicate comparison independent of NaN and signed-zero equality.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct MastRow { pub source_id: u64, pub designation: Option<String>, pub values: [Option<u64>; 11] }
impl MastRow {
    pub fn numbers(&self) -> [Option<f64>; 11] { self.values.map(|v| v.map(f64::from_bits)) }
}

#[derive(Clone)]
pub(super) struct Ranges { pub size: u64, pub parts: Vec<(u64, Bytes)> }
impl Length for Ranges { fn len(&self) -> u64 { self.size } }
impl ChunkReader for Ranges {
    type T = Cursor<Bytes>;
    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        self.parts.iter().find_map(|(offset, bytes)| start.checked_sub(*offset).filter(|&n| n < bytes.len() as u64).map(|n| Cursor::new(bytes.slice(n as usize..))))
            .ok_or_else(|| parquet::errors::ParquetError::General(format!("unfetched Parquet offset {start}")))
    }
    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
        let reader = self.get_read(start)?;
        if reader.get_ref().len() < length { return Err(parquet::errors::ParquetError::General("read extends beyond selected range".into())); }
        Ok(reader.into_inner().slice(..length))
    }
}
pub(super) fn parse_footer(bytes: &[u8]) -> io::Result<ParquetMetaData> {
    if bytes.len() < 8 || &bytes[bytes.len()-4..] != b"PAR1" { return Err(invalid("invalid Parquet footer magic")); }
    let n = u32::from_le_bytes(bytes[bytes.len()-8..bytes.len()-4].try_into().unwrap()) as usize;
    if n > crate::constants::MAST_FOOTER_MAX_BYTES || n + 8 > bytes.len() { return Err(invalid("invalid Parquet footer length")); }
    ParquetMetaDataReader::decode_metadata(&bytes[bytes.len()-8-n..bytes.len()-8]).map_err(io::Error::other)
}
pub(super) fn projected_schema(metadata: &ParquetMetaData) -> io::Result<Type> {
    let schema = metadata.file_metadata().schema();
    let mut selected = Vec::new();
    for (index, name) in COLUMNS.iter().enumerate() {
        let matches: Vec<_> = schema.get_fields().iter().filter(|field| field.name() == *name).collect();
        if matches.len() != 1 { return Err(invalid(format!("missing/duplicate Gaia column {name}"))); }
        let field = matches[0];
        if !field.is_primitive() || field.get_basic_info().repetition() == parquet::basic::Repetition::REPEATED { return Err(invalid(format!("Gaia column {name} must be scalar"))); }
        let kind = field.get_physical_type();
        let valid = match index { 0 => kind == Physical::INT64, 1 => kind == Physical::BYTE_ARRAY, _ => matches!(kind, Physical::FLOAT | Physical::DOUBLE) };
        if !valid { return Err(invalid(format!("incompatible Gaia datatype for {name}"))); }
        selected.push(Arc::clone(field));
    }
    Type::group_type_builder(schema.name()).with_fields(selected).build().map_err(io::Error::other)
}
pub(super) fn selected_ranges(metadata: &ParquetMetaData, group: usize, size: u64) -> io::Result<Vec<(u64, usize)>> {
    projected_schema(metadata)?;
    let rg = metadata.row_group(group);
    let mut ranges = Vec::new();
    let (mut compressed, mut decoded) = (0usize, 0usize);
    let count = usize::try_from(rg.num_rows()).map_err(io::Error::other)?;
    if count > MAST_DECODED_MAX_BYTES / std::mem::size_of::<MastRow>() { return Err(invalid("too many rows in MAST group")); }
    for name in COLUMNS {
        let c = rg.columns().iter().find(|c| c.column_path().string() == name).ok_or_else(|| invalid("missing scalar column chunk"))?;
        let start = u64::try_from(c.dictionary_page_offset().unwrap_or(c.data_page_offset())).map_err(|_| invalid("negative Parquet column offset"))?;
        let n = usize::try_from(c.compressed_size()).map_err(|_| invalid("negative Parquet column size"))?;
        if n == 0 && count > 0 { return Err(invalid("empty column chunk in nonempty group")); }
        if start.checked_add(n as u64).is_none_or(|end| end > size) { return Err(invalid("column extends beyond remote file")); }
        compressed = compressed.checked_add(n).ok_or_else(|| invalid("column size overflow"))?;
        decoded = decoded.checked_add(usize::try_from(c.uncompressed_size()).map_err(io::Error::other)?).ok_or_else(|| invalid("decoded size overflow"))?;
        ranges.push((start,n));
    }
    if compressed > MAST_BLOCK_MAX_BYTES || decoded > MAST_DECODED_MAX_BYTES { return Err(invalid("MAST row group exceeds byte budget")); }
    Ok(ranges)
}
pub(super) fn read_group<R: ChunkReader + 'static>(input: R, group: usize) -> io::Result<Vec<MastRow>> {
    let size = input.len();
    let reader = SerializedFileReader::new(input).map_err(io::Error::other)?;
    if group >= reader.num_row_groups() { return Err(invalid("invalid Parquet group")); }
    let schema = projected_schema(reader.metadata())?;
    selected_ranges(reader.metadata(), group, size)?;
    let group = reader.get_row_group(group).map_err(io::Error::other)?;
    let expected = usize::try_from(group.metadata().num_rows()).map_err(io::Error::other)?;
    let rows = group.get_row_iter(Some(schema)).map_err(io::Error::other)?.with_batch_size(MAST_DECODE_BATCH_ROWS);
    let mut output = Vec::new(); output.try_reserve(expected).map_err(io::Error::other)?;
    let mut bytes = expected * std::mem::size_of::<MastRow>();
    for row in rows {
        let fields = row.map_err(io::Error::other)?.into_columns();
        let mut result = MastRow { source_id: 0, designation: None, values: [None;11] };
        for (name, field) in fields {
            let index = COLUMNS.iter().position(|c| *c == name).ok_or_else(|| invalid("unexpected projected field"))?;
            match (index, field) {
                (0, Field::Long(id)) if id >= 0 => result.source_id = id as u64,
                (0, _) => return Err(invalid("missing/invalid Gaia source_id")),
                (1, Field::Str(label)) => { bytes = bytes.checked_add(label.len()).ok_or_else(|| invalid("label size overflow"))?; result.designation = Some(label); },
                (1, Field::Null) => {},
                (1, _) => return Err(invalid("designation must be UTF-8")),
                (i, Field::Null) => result.values[i-2] = None,
                (i, Field::Double(v)) => result.values[i-2] = Some(v.to_bits()),
                (i, Field::Float(v)) => result.values[i-2] = Some(f64::from(v).to_bits()),
                _ => return Err(invalid("unexpected Gaia scalar type")),
            }
        }
        if bytes > MAST_DECODED_MAX_BYTES || output.len() >= expected { return Err(invalid("decoded Gaia block exceeds bounds")); }
        output.push(result);
    }
    if output.len() != expected { return Err(invalid("truncated Gaia row group")); }
    Ok(output)
}
pub(crate) fn read_block(bytes: Vec<u8>) -> io::Result<Vec<MastRow>> { read_group(Bytes::from(bytes), 0) }

pub(super) fn write_block(rows: &[MastRow], schema: Type) -> io::Result<Vec<u8>> {
    if rows.iter().any(|r| r.source_id > i64::MAX as u64) { return Err(invalid("source_id exceeds signed Gaia range")); }
    let fields = schema.get_fields().iter().map(|f| {
        Type::primitive_type_builder(f.name(), f.get_physical_type()).with_repetition(parquet::basic::Repetition::OPTIONAL)
            .with_converted_type(f.get_basic_info().converted_type()).with_logical_type(f.get_basic_info().logical_type_ref().cloned()).build().map(Arc::new)
    }).collect::<Result<Vec<_>,_>>().map_err(io::Error::other)?;
    let schema = Type::group_type_builder("gaia").with_fields(fields).build().map_err(io::Error::other)?;
    let props = WriterProperties::builder().set_compression(Compression::ZSTD(Default::default())).build();
    let mut writer = SerializedFileWriter::new(Vec::new(), Arc::new(schema), Arc::new(props)).map_err(io::Error::other)?;
    let mut group = writer.next_row_group().map_err(io::Error::other)?;
    write_columns(rows, &mut group)?;
    group.close().map_err(io::Error::other)?;
    writer.into_inner().map_err(io::Error::other)
}
fn write_columns<W: io::Write + Send>(rows: &[MastRow], group: &mut parquet::file::writer::SerializedRowGroupWriter<'_, W>) -> io::Result<()> {
    for i in 0..COLUMNS.len() {
        let mut column = group.next_column().map_err(io::Error::other)?.ok_or_else(|| invalid("missing output column"))?;
        let defs: Vec<i16> = rows.iter().map(|r| if i == 0 || if i==1 { r.designation.is_some() } else { r.values[i-2].is_some() } {1} else {0}).collect();
        match i {
            0 => { let values: Vec<i64> = rows.iter().map(|r| r.source_id as i64).collect(); column.typed::<Int64Type>().write_batch(&values, Some(&defs), None).map_err(io::Error::other)?; },
            1 => { let values: Vec<ByteArray> = rows.iter().filter_map(|r| r.designation.as_ref().map(|s| ByteArray::from(s.as_str()))).collect(); column.typed::<ByteArrayType>().write_batch(&values, Some(&defs), None).map_err(io::Error::other)?; },
            _ => {
                let values: Vec<f64> = rows.iter().filter_map(|r| r.values[i-2].map(f64::from_bits)).collect();
                match column.untyped() {
                    parquet::column::writer::ColumnWriter::FloatColumnWriter(w) => { w.write_batch(&values.iter().map(|v| *v as f32).collect::<Vec<_>>(), Some(&defs), None).map_err(io::Error::other)?; },
                    parquet::column::writer::ColumnWriter::DoubleColumnWriter(w) => { w.write_batch(&values, Some(&defs), None).map_err(io::Error::other)?; },
                    _ => return Err(invalid("invalid output numeric type")),
                }
            }
        }
        column.close().map_err(io::Error::other)?;
    }
    Ok(())
}
