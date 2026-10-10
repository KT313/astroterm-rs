use super::*;
use crate::progress::NoProgress;
use ::parquet::schema::parser::parse_message_type;
use std::sync::{Mutex, atomic::{AtomicBool, Ordering}};

fn row(id:u64)->parquet::MastRow {
    parquet::MastRow{source_id:id,designation:Some(format!("Gaia DR3 {id}")),values:[45.,0.,2016.,1.,2.,1.,20.,0.,12.,13.,11.].map(|v|Some(f64::to_bits(v)))}
}
fn schema()->::parquet::schema::types::Type {
    parse_message_type("message gaia { OPTIONAL INT64 source_id; OPTIONAL BYTE_ARRAY designation (UTF8); OPTIONAL DOUBLE ra; OPTIONAL DOUBLE dec; OPTIONAL DOUBLE ref_epoch; OPTIONAL DOUBLE pmra; OPTIONAL DOUBLE pmdec; OPTIONAL DOUBLE parallax; OPTIONAL FLOAT parallax_over_error; OPTIONAL FLOAT radial_velocity; OPTIONAL FLOAT phot_g_mean_mag; OPTIONAL FLOAT phot_bp_mean_mag; OPTIONAL FLOAT phot_rp_mean_mag; }").unwrap()
}
struct Server {calls:Mutex<Vec<String>>, offline:AtomicBool, changed:AtomicBool, data:Vec<u8>}
impl Server {fn new()->Self {Self{calls:Mutex::new(Vec::new()),offline:AtomicBool::new(false),changed:AtomicBool::new(false),data:parquet::write_block(&[row(10),row(20),row(30)],schema()).unwrap()}}}
impl Fetch for Server {
    fn fetch(&self,path:&str,range:Option<&str>,identity:Option<&Identity>,limit:usize,_:Instant)->io::Result<super::super::transport::Response> {
        if self.offline.load(Ordering::Relaxed) {return Err(io::Error::other("offline"));}
        self.calls.lock().unwrap().push(format!("{path}:{range:?}"));
        if path=="properties" {return Ok(super::super::transport::Response{bytes:b"obs_collection=gaia\ndataproduct_type=object\n".to_vec(),identity:None,start:0});}
        if path=="partition_info.csv" {return Ok(super::super::transport::Response{bytes:b"Norder,Npix\n0,0\n0,1\n".to_vec(),identity:None,start:0});}
        let id=Identity{etag:format!("\"{path}:{}\"",self.changed.load(Ordering::Relaxed)),size:self.data.len() as u64};
        if identity.is_some_and(|expected|expected!=&id) {return Err(io::Error::other("changed"));}
        let range=range.unwrap().strip_prefix("bytes=").unwrap();
        let (a,b)=range.split_once('-').unwrap();
        let (a,b)=if a.is_empty() {(self.data.len().saturating_sub(b.parse().unwrap()),self.data.len()-1)} else {(a.parse().unwrap(),b.parse().unwrap())};
        let bytes=self.data[a..=b].to_vec();assert!(bytes.len()<=limit);
        Ok(super::super::transport::Response{bytes,identity:Some(id),start:a as u64})
    }
}
#[test]
fn mast_prefix_extension_shrink_and_offline_reuse_only_fetch_missing_groups() {
    let dir=tempfile::tempdir().unwrap();let store=BlobStore::new(dir.path().into());let server=Server::new();let mut p=NoProgress(Vec::new());let mut times=StepTimes::default();
    let first=resolve_selection(2,store.clone(),&server,&mut p,&mut times).unwrap();
    assert_eq!((first.blocks.len(),first.blocks[0].selected,first.available),(1,2,3));
    let calls=server.calls.lock().unwrap().len();
    server.offline.store(true,Ordering::Relaxed);
    for n in [1,2,3] {assert_eq!(resolve_selection(n,store.clone(),&server,&mut p,&mut times).unwrap().blocks[0].selected,n as usize);}
    assert_eq!(server.calls.lock().unwrap().len(),calls);
    assert!(resolve_selection(4,store.clone(),&server,&mut p,&mut times).is_err());
    server.offline.store(false,Ordering::Relaxed);
    let next=resolve_selection(4,store.clone(),&server,&mut p,&mut times).unwrap();
    assert_eq!((next.blocks.len(),next.blocks[1].selected,next.blocks[1].base),(2,1,3));
    assert_eq!(next.blocks[0].hash,first.blocks[0].hash);
    assert!(server.calls.lock().unwrap()[calls..].iter().all(|c|c.contains("Npix=1")));
    server.offline.store(true,Ordering::Relaxed);
    let all=resolve_selection(99,store,&server,&mut p,&mut times).unwrap();assert_eq!(all.available,6);
    assert!(String::from_utf8(p.0).unwrap().contains("contains only 6"));
}
#[test]
fn mast_projection_preserves_nulls_strings_and_exact_ids_and_nan_bits() {
    let mut a=row((1<<54)+1);a.designation=Some("label with Unicode α".into());a.values[3]=None;
    let mut b=row(77);b.values[0]=Some(f64::NAN.to_bits());b.values[1]=Some((-0.0f64).to_bits());
    let raw=parquet::write_block(&[a.clone(),b.clone()],schema()).unwrap();
    assert_eq!(parquet::read_block(raw).unwrap(),vec![a,b]);
}
#[test]
fn mast_discovery_rejects_bad_and_overlapping_leaves() {
    for bad in ["Norder,Npix\n0,0\n0,0\n","Norder,Npix\n0,0\n1,1\n","Norder,Npix\n30,0\n","Norder,Npix\n0,12\n","wrong,Npix\n0,0\n"] {assert!(parse_leaves(bad).is_err(),"{bad}");}
    assert_eq!(parse_leaves("Norder,Npix\n1,5\n0,0\n").unwrap(),vec![Leaf{order:0,pixel:0},Leaf{order:1,pixel:5}]);
}
#[test]
fn mast_range_reader_refuses_unfetched_columns_and_bad_footer() {
    let bytes=parquet::write_block(&[row(10)],schema()).unwrap();let n=bytes.len();
    assert!(parquet::parse_footer(&bytes[..n-1]).is_err());
    let metadata=parquet::parse_footer(&bytes).unwrap();let ranges=parquet::selected_ranges(&metadata,0,n as u64).unwrap();
    assert_eq!(ranges.len(),13);
    let tail=u32::from_le_bytes(bytes[n-8..n-4].try_into().unwrap()) as usize+8;
    let input=Ranges{size:n as u64,parts:vec![((n-tail) as u64,Bytes::copy_from_slice(&bytes[n-tail..]))]};
    assert!(parquet::read_group(input,0).is_err());
}

#[test]
fn mast_corrupt_raw_block_repairs_independently_but_changed_remote_identity_fails() {
    let dir = tempfile::tempdir().unwrap();
    let store = BlobStore::new(dir.path().into());
    let server = Server::new();
    let mut progress = NoProgress(Vec::new());
    let mut times = StepTimes::default();
    let initial = resolve_selection(4, store.clone(), &server, &mut progress, &mut times).unwrap();
    let broken = dir.path().join("objects").join(&initial.blocks[0].hash);
    std::fs::write(&broken, b"interrupted/corrupt local data").unwrap();
    server.changed.store(true, Ordering::Relaxed);
    let error = resolve_selection(4, store.clone(), &server, &mut progress, &mut times).err().expect("changed source must fail");
    assert!(error.to_string().contains("changed"));
    server.changed.store(false, Ordering::Relaxed);
    let repaired = resolve_selection(4, store, &server, &mut progress, &mut times).unwrap();
    assert_eq!(repaired.blocks[1].hash, initial.blocks[1].hash);
    assert_eq!(repaired.blocks[0].hash, initial.blocks[0].hash);
}
