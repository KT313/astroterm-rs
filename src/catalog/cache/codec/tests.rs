use super::*;
fn sink() -> crate::progress::NoProgress<io::Sink> { crate::progress::NoProgress(io::sink()) }

#[test]
fn streaming_chunks_preserve_bits_and_cross_element_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cache");
    let input: Vec<[u32; 3]> = (0..CACHE_CHUNK_BYTES / 12 + 5).map(|i| [i as u32, !i as u32, 0]).collect();
    for length in [0, 1, CACHE_CHUNK_BYTES, CACHE_CHUNK_BYTES + 1] {
        let bytes = vec![17; length];
        write_sections(&path, &[4; 32], &[&bytes, bytemuck::cast_slice(&input)], &mut sink()).unwrap();
        let mut reader = PreparedCatalogReader::open(&path, &[4; 32], &mut sink()).unwrap();
        assert_eq!(reader.decode::<u8>(0, &mut sink()).unwrap(), bytes);
        assert_eq!(reader.decode::<[u32; 3]>(1, &mut sink()).unwrap(), input);
        reader.finish(&mut sink()).unwrap();
    }
}

#[test]
fn corrupt_headers_lengths_frames_and_trailers_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cache");
    write_sections(&path, &[7; 32], &[&[42; 128]], &mut sink()).unwrap();
    let original = std::fs::read(&path).unwrap();
    for case in 0..10 {
        let mut bytes = original.clone();
        let chunk = PREFIX + 16;
        match case {
            0 => bytes.truncate(10),
            1 => bytes[8..12].copy_from_slice(&8u32.to_le_bytes()),
            2 => bytes[16] ^= 1,
            3 => bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes()),
            4 => bytes[chunk..chunk + 8].copy_from_slice(&u64::MAX.to_le_bytes()),
            5 => bytes[chunk + 8] ^= 1,
            6 => { bytes.pop(); },
            7 => bytes.push(0),
            8 => bytes[PREFIX + 8..PREFIX + 16].copy_from_slice(&127u64.to_le_bytes()),
            _ => { let n = bytes.len(); bytes[n - 1] ^= 1; },
        }
        std::fs::write(&path, bytes).unwrap();
        let result = (|| {
            let mut reader = PreparedCatalogReader::open(&path, &[7; 32], &mut sink())?;
            reader.decode::<u8>(0, &mut sink())?;
            reader.finish(&mut sink())
        })();
        assert!(result.is_err(), "case {case}");
    }
}

#[test]
fn chunk_padding_extra_frames_and_incorrect_directory_sizes_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cache");
    write_sections(&path, &[3; 32], &[&[1; 100]], &mut sink()).unwrap();
    let original = std::fs::read(&path).unwrap();
    for case in 0..4 {
        let mut bytes = original.clone();
        let start = PREFIX + 16;
        let length = u64_at(&bytes, start) as usize;
        match case {
            0 => {
                bytes.insert(start + 8 + length, 0);
                bytes[start..start + 8].copy_from_slice(&((length + 1) as u64).to_le_bytes());
            },
            1 => {
                let frame = bytes[start + 8..start + 8 + length].to_vec();
                bytes.splice(start + 8 + length..start + 8 + length, frame);
                bytes[start..start + 8].copy_from_slice(&((length * 2) as u64).to_le_bytes());
            },
            2 => bytes[PREFIX + 8..PREFIX + 16].copy_from_slice(&u64::MAX.to_le_bytes()),
            _ => bytes[PREFIX..PREFIX + 8].copy_from_slice(&1u64.to_le_bytes()),
        }
        let end = bytes.len() - 4;
        let checksum = crc32fast::hash(&bytes[..end]);
        bytes[end..].copy_from_slice(&checksum.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        let result = (|| {
            let mut reader = PreparedCatalogReader::open(&path, &[3; 32], &mut sink())?;
            reader.decode::<u8>(0, &mut sink())?;
            reader.finish(&mut sink())
        })();
        assert!(result.is_err(), "case {case}");
    }
}

#[test]
fn publication_failure_keeps_existing_destination_and_write_errors_propagate() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("occupied");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("keep"), b"unchanged").unwrap();
    assert!(write_sections(&path, &[0; 32], &[b"test"], &mut sink()).is_err());
    assert_eq!(std::fs::read(path.join("keep")).unwrap(), b"unchanged");
    struct Failing;
    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> { Err(io::Error::other("injected write failure")) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    assert!(write_hashed(&mut Failing, &mut crc32fast::Hasher::new(), b"chunk").is_err());
}

#[test]
fn concurrent_publications_leave_one_complete_stream() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cache");
    std::thread::scope(|scope| {
        for value in [19u8, 211] {
            let path = &path;
            scope.spawn(move || write_sections(path, &[9; 32], &[&vec![value; 10000]], &mut sink()).unwrap());
        }
    });
    let mut reader = PreparedCatalogReader::open(&path, &[9; 32], &mut sink()).unwrap();
    let output = reader.decode::<u8>(0, &mut sink()).unwrap();
    reader.finish(&mut sink()).unwrap();
    assert!([19, 211].contains(&output[0]));
    assert!(output.iter().all(|&value| value == output[0]));
}
