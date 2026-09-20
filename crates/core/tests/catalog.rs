use soundshelf_core::catalog::*;
use std::{fs, path::Path};
use tempfile::tempdir;

fn profile() -> Profile { Profile {duration:1.0,sample_rate:48000,channels:1,frames:48000,peak:0.8,rms:0.2,description:"Measured".into(),tags:vec!["short".into()],waveform:vec![[-0.8,0.8]]} }
fn db() -> Catalog { Catalog::open(Path::new(":memory:")).unwrap() }
fn add(c: &mut Catalog, root: &Path) -> (Source,String,String) {
    fs::write(root.join("test.wav"),b"test audio").unwrap();
    let s=c.add_source(root).unwrap(); let h=hash_file(&root.join("test.wav")).unwrap();
    let id=c.register(&s,"test.wav",&h).unwrap(); c.publish(&s,&id,&h,&profile()).unwrap(); (s,id,h)
}
#[test] fn relink_reuses_identity_and_profile() {
    let base=tempdir().unwrap(); let old=base.path().join("old");fs::create_dir(&old).unwrap();
    let mut c=db(); let(s,id,h)=add(&mut c,&old);c.annotate(&id,&["My_Tag".into()],"use in scene",true).unwrap();
    let new=base.path().join("moved");fs::rename(&old,&new).unwrap();let moved=c.relink(&s.id,&new).unwrap();
    assert_eq!(moved.generation,1); assert_eq!(c.register(&moved,"test.wav",&h).unwrap(),id);
    let sound=c.sound(&id).unwrap();assert_eq!(sound.profile,Some(profile()));assert_eq!(sound.user_tags,vec!["My Tag"]);assert_eq!(sound.comment,"use in scene");assert!(sound.favorite);
    assert_eq!(c.resolve(&id).unwrap(),new.join("test.wav").canonicalize().unwrap());
    assert!(c.publish(&s,&id,&h,&profile()).is_err());
}
#[test] fn changed_content_keeps_annotations_but_not_stale_measurements() {
    let root=tempdir().unwrap(); let mut c=db();let(s,id,_)=add(&mut c,root.path());c.annotate(&id,&["impact".into()],"mine",false).unwrap();
    c.register(&s,"test.wav","new digest").unwrap();let sound=c.sound(&id).unwrap();assert_eq!(sound.status,"pending");assert!(sound.profile.is_none());assert_eq!(sound.comment,"mine");
}
#[test] fn wrong_root_is_atomic() {
    let root=tempdir().unwrap();let bad=tempdir().unwrap();let mut c=db();let(s,id,_)=add(&mut c,root.path());fs::write(bad.path().join("test.wav"),b"different").unwrap();assert!(c.relink(&s.id,bad.path()).is_err());assert_eq!(c.source(&s.id).unwrap(),s);assert!(c.resolve(&id).is_ok());
}
#[test] fn incomplete_scan_does_not_delete() {
    let root=tempdir().unwrap();let mut c=db();let(s,id,_)=add(&mut c,root.path());c.reconcile(&s,&[],false).unwrap();assert_eq!(c.sound(&id).unwrap().status,"ready");c.reconcile(&s,&[],true).unwrap();assert_eq!(c.sound(&id).unwrap().status,"missing");assert!(c.resolve(&id).is_err());
}
#[test] fn offline_source_is_not_deleted() {let root=tempdir().unwrap();let mut c=db();let(s,id,_)=add(&mut c,root.path());c.set_available(&s.id,false).unwrap();assert!(c.resolve(&id).is_err());assert_eq!(c.sound(&id).unwrap().status,"ready");}
#[test] fn paths_reject_traversal() {for p in ["", "../secret", "/tmp/a", "a/../b", "a//b", "C:\\x", "a\\b", "./a"] {assert!(valid_relative(p).is_err(),"{p}");}assert!(valid_relative("folder/hello sound.wav").is_ok());}
#[cfg(unix)] #[test] fn symlink_escape_rejected() {let root=tempdir().unwrap();let out=tempdir().unwrap();fs::write(out.path().join("a"),b"x").unwrap();std::os::unix::fs::symlink(out.path().join("a"),root.path().join("a")).unwrap();assert!(contained(root.path(),"a").is_err());}
#[test] fn duplicate_roots_idempotent_and_overlap_denied() {let root=tempdir().unwrap();let c=db();let a=c.add_source(root.path()).unwrap();assert_eq!(c.add_source(root.path()).unwrap().id,a.id);fs::create_dir(root.path().join("nested")).unwrap();assert!(c.add_source(&root.path().join("nested")).is_err());}
#[test] fn metadata_persists_and_normalizes_tags() {let root=tempdir().unwrap();let path=root.path().join("library.db");let media=tempdir().unwrap();let mut c=Catalog::open(&path).unwrap();let(_,id,_)=add(&mut c,media.path());c.annotate(&id,&[" high_pitch ".into(),"HIGH PITCH".into()],"<script>plain text</script>",true).unwrap();drop(c);let c=Catalog::open(&path).unwrap();let s=c.sound(&id).unwrap();assert_eq!(s.user_tags,vec!["high pitch"]);assert_eq!(s.comment,"<script>plain text</script>");assert!(s.favorite);}
#[test] fn invalid_profiles_do_not_publish() {let root=tempdir().unwrap();let mut c=db();let(s,id,h)=add(&mut c,root.path());let mut p=profile();p.rms=f32::NAN;assert!(c.publish(&s,&id,&h,&p).is_err());assert_eq!(c.sound(&id).unwrap().profile,Some(profile()));}
