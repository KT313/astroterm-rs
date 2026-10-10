//! Opt-in real transport qualification. Ordinary tests never download data.
use astroterm::{catalog::datasets::{Dataset,DatasetDirectories}, sky::load_sky_catalog_with_times, timing::StepTimes, progress::{DatasetProgress,Stage,Unit}};
use std::{io::{self,Write},path::PathBuf};

#[derive(Default)]
struct Counts { downloaded: usize, converted: usize, text: Vec<u8> }
impl Write for Counts {
    fn write(&mut self,b:&[u8])->io::Result<usize> {self.text.write(b)}
    fn flush(&mut self)->io::Result<()> {Ok(())}
}
impl DatasetProgress for Counts {
    fn begin_stage(&mut self,stage:Stage,detail:&str,_:Unit,_:Option<u64>) {
        if stage==Stage::Download {self.downloaded+=1;}
        if stage==Stage::Convert && detail=="MAST block" {self.converted+=1;}
        eprintln!("{}: {detail}",stage.label());
    }
}
#[test]
#[ignore = "requires explicit ASTROTERM_MAST_TEST_ROOT and network; at most the first two row groups"]
fn mast_two_groups_extend_without_reconverting_old_rows() {
    let root=PathBuf::from(std::env::var_os("ASTROTERM_MAST_TEST_ROOT").expect("set an isolated empty test data directory"));
    let dirs=DatasetDirectories{data:Some(root.join("data")),cache:Some(root.join("cache"))};
    let mut first_ids=Vec::new();
    for (n,expected_downloads,expected_conversions) in [(100000,1,1),(150000,0,0),(200001,1,1),(100000,0,0)] {
        let mut log=Counts::default();let started=std::time::Instant::now();
        let catalog=load_sky_catalog_with_times(Some(&Dataset::GaiaGdr3Stars{stars:n}),&dirs,&mut log,&mut StepTimes::default()).unwrap();
        assert_eq!((log.downloaded,log.converted),(expected_downloads,expected_conversions));
        let mut ids:Vec<_>=catalog.catalog.stars.iter().map(|s|s.id.0).collect();ids.sort_unstable();
        if first_ids.is_empty() {first_ids=ids.clone();} else {assert!(first_ids.iter().all(|id|ids.binary_search(id).is_ok()));}
        assert!(catalog.catalog.stars.iter().all(|s|catalog.catalog.names.get(s.name).unwrap().starts_with("Gaia DR3 ")));
        eprintln!("N={n}; loaded={}; download blocks={}; converted blocks={}; elapsed={:?}\n{}",ids.len(),log.downloaded,log.converted,started.elapsed(),String::from_utf8_lossy(&log.text));
    }
}
