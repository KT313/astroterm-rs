use super::*;
use crate::{catalog::StarNames, progress::NoProgress};

fn row(id:u64)->MastRow {MastRow{source_id:id,designation:Some(format!("supplied label {id}")),values:[45.,0.,2016.,1.,2.,1.,20.,0.,12.,13.,11.].map(|v|Some(f64::to_bits(v)))}}
fn selection(dir:&std::path::Path,groups:&[Vec<MastRow>],count:usize)->(MastSelection,BlobStore,Vec<String>) {
    let raw=BlobStore::new(dir.join("raw"));let converted=BlobStore::new(dir.join("converted"));let policy=fingerprint();let mut base=0u64;let mut blocks=Vec::new();let mut keys=Vec::new();
    for (i,rows) in groups.iter().enumerate() {
        if base>=count as u64 {break;}
        let key=format!("block{i}");let bytes=vec![i as u8];raw.publish(&key,&bytes,false).unwrap();
        let block=MastBlock{key,hash:mast_digest(&bytes),rows:rows.len(),selected:rows.len().min(count-base as usize),base};
        let key=block_key(&block,&policy);converted.publish(&key,&encode_mast(&convert_block(rows).unwrap()).unwrap(),false).unwrap();
        base+=rows.len() as u64;blocks.push(block);keys.push(key);
    }
    (MastSelection{store:raw,blocks,discovery:"test".into(),requested:count as u32,available:base},converted,keys)
}
#[test]
fn mast_converted_prefix_reuses_results_and_deduplicates_across_blocks() {
    let dir=tempfile::tempdir().unwrap();let a=row(10);let b=row(20);let c=row(30);
    let groups=vec![vec![a,b.clone()],vec![b.clone(),b,c]];
    for (n,expected) in [(1,vec![0]),(2,vec![0,1]),(3,vec![0,1]),(4,vec![0,1]),(5,vec![0,1,4])] {
        let (sel,store,keys)=selection(dir.path(),&groups,n);let mut progress=NoProgress(Vec::new());
        // Source blobs deliberately are not Parquet: a successful assembly proves conversion-cache reuse.
        let catalog=super::super::assembly::assemble_selected_catalog(&sel,&store,&keys,&mut progress,&mut StepTimes::default()).unwrap();
        let mut ids:Vec<_>=catalog.catalog.stars.iter().map(|s|s.id.0).collect();ids.sort_unstable();assert_eq!(ids,expected);
        for star in catalog.catalog.stars.iter() {assert!(catalog.catalog.names.get(star.name).unwrap().starts_with("supplied label"));}
        if n==5 {assert!(String::from_utf8(progress.0).unwrap().contains("duplicate copies removed=2"));}
    }
}
#[test]
fn mast_overfetched_error_is_deferred_until_selected_and_conflicts_fail() {
    let dir=tempfile::tempdir().unwrap();let mut bad=row(20);bad.values[0]=Some(f64::NAN.to_bits());let groups=vec![vec![row(10),bad]];
    for n in [1,2] {
        let (sel,store,keys)=selection(dir.path(),&groups,n);
        let result=super::super::assembly::assemble_selected_catalog(&sel,&store,&keys,&mut NoProgress(Vec::new()),&mut StepTimes::default());
        assert_eq!(result.is_ok(),n==1);
        if n==2 {assert!(result.unwrap_err().to_string().contains("nonfinite"));}
    }
    let mut conflict=row(10);conflict.designation=Some("different".into());
    let data=convert_block(&[row(10),conflict]).unwrap();assert!(matches!(data.entries[1].result,ConvertedRow::Error{..}));
    let (sel,store,keys)=selection(dir.path(),&[vec![row(10),row(20),row(10)]],3);
    assert!(super::super::assembly::assemble_selected_catalog(&sel,&store,&keys,&mut NoProgress(Vec::new()),&mut StepTimes::default()).unwrap_err().to_string().contains("nonadjacent"));
}
#[test]
fn mast_duplicate_float_comparison_uses_bits_but_nan_still_fails_validation() {
    let mut nan=row(1);nan.values[0]=Some(0x7ff8_0000_0000_0042);
    let converted=convert_block(&[nan.clone(),nan]).unwrap();assert!(matches!(converted.entries[0].result,ConvertedRow::Error{..}));assert!(matches!(converted.entries[1].result,ConvertedRow::Duplicate));
    let a=row(1);let mut b=a.clone();b.values[1]=Some((-0.0f64).to_bits());
    assert!(matches!(convert_block(&[a,b]).unwrap().entries[1].result,ConvertedRow::Error{..}));
}
#[test]
fn mast_packed_append_matches_existing_storage_preparation() {
    let source=validate_gaia_input(StarId(0),10,row(10).numbers()).unwrap().unwrap();
    let mut star=convert_source(source,&mut ConversionStats::default()).unwrap();let mut names=StarNames::default();star.name=Some(names.insert("label").unwrap());
    let mut old=StarStorage::default();let bound=old.push(star.clone()).unwrap();
    let (packed,new_bound,clip)=StarStorage::prepare_row(star).unwrap();let mut reused=StarStorage::default();reused.push_prepared(packed,clip);
    assert_eq!(bound.to_bits(),new_bound.to_bits());assert_eq!(old,reused);
}

#[test]
#[ignore = "local sample microbenchmark; requires ASTROTERM_MAST_SAMPLE, no network"]
fn mast_measure_compact_preparation_and_labels() {
    use std::{time::Instant,hint::black_box};
    let path=std::env::var_os("ASTROTERM_MAST_SAMPLE").expect("set a normalized 13-column Parquet sample");
    let rows=read_mast_block(std::fs::read(path).unwrap()).unwrap();
    let mut stats=ConversionStats::default();let mut stars=Vec::new();let mut labels=Vec::new();
    for (i,row) in rows.iter().enumerate() {
        if let Some(source)=validate_gaia_input(StarId(i as u32),row.source_id,row.numbers()).unwrap() {
            stars.push(convert_source(source,&mut stats).unwrap());labels.push(row.designation.as_ref().unwrap());
        }
    }
    let started=Instant::now();
    for star in &stars {black_box(StarStorage::prepare_row(star.clone()).unwrap());}
    let prepare=started.elapsed();let started=Instant::now();let mut names=StarNames::default();
    for label in &labels {black_box(names.insert(label).unwrap());}
    eprintln!("usable={}; compact preparation={prepare:?}; label insertion={:?}",stars.len(),started.elapsed());
}
