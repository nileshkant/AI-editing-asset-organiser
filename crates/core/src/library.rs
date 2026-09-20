use crate::{catalog::{Catalog,Source,hash_file,path_text,contained},media::{MediaTools,analyze},invalid,Result};
use serde::{Serialize,Deserialize};
use std::{path::Path,sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}}};
use walkdir::WalkDir;

#[derive(Clone,Default,Debug,Serialize,Deserialize)]
pub struct Progress {pub source_id:String,pub status:String,pub completed:usize,pub total:usize,pub reused:usize,pub failed:usize,pub current:String,pub errors:Vec<String>}

pub fn scan(catalog:Arc<Mutex<Catalog>>,source:Source,tools:&MediaTools,cancel:Arc<AtomicBool>,mut progress:impl FnMut(Progress))->Result<Progress>{
    tools.validate()?;
    let root=Path::new(&source.root);let mut state=Progress{source_id:source.id.clone(),status:"discovering".into(),..Default::default()};progress(state.clone());
    if !root.is_dir(){catalog.lock().map_err(|_|invalid("Catalog unavailable"))?.set_available(&source.id,false)?;return Err(invalid("Source is offline or inaccessible"));}
    catalog.lock().map_err(|_|invalid("Catalog unavailable"))?.set_available(&source.id,true)?;
    let mut files=vec![];let mut complete=true;
    for entry in WalkDir::new(root).follow_links(false).into_iter().filter_entry(|e|!e.file_name().to_string_lossy().starts_with('.')){
        if cancel.load(Ordering::Relaxed){return Err(invalid("Scan cancelled"));}
        match entry{Ok(e) if e.file_type().is_file() && supported(e.path())=>files.push(e.path().to_owned()),Ok(_)=>{},Err(e)=>{complete=false;if state.errors.len()<50{state.errors.push(e.to_string());}}}
    }
    files.sort();state.total=files.len();state.status="analyzing".into();progress(state.clone());let mut seen=vec![];
    for file in files {
        if cancel.load(Ordering::Relaxed){return Err(invalid("Scan cancelled"));}
        let relative=path_text(file.strip_prefix(root).map_err(|_|invalid("Invalid source path"))?)?.replace(std::path::MAIN_SEPARATOR,"/");state.current=relative.clone();progress(state.clone());
        let result=(||->Result<()> {
            let file=contained(root,&relative)?;
            let hash=hash_file(&file)?;
            // Reconciliation may only preserve a path after it was verified and hashed.
            // A file deleted between discovery and this point must become missing.
            seen.push(relative.clone());
            let id={catalog.lock().map_err(|_|invalid("Catalog unavailable"))?.register(&source,&relative,&hash)?};
            if catalog.lock().map_err(|_|invalid("Catalog unavailable"))?.cached_profile(&hash)?.is_some(){state.reused+=1;return Ok(());}
            match analyze(tools,&file,cancel.clone()) {
                Ok(profile)=>{if hash_file(&file)?!=hash{return Err(invalid("File changed while being analyzed"));}catalog.lock().map_err(|_|invalid("Catalog unavailable"))?.publish(&source,&id,&hash,&profile)?;},
                Err(error)=>{catalog.lock().map_err(|_|invalid("Catalog unavailable"))?.set_status(&id,"failed")?;return Err(error);}
            }Ok(())
        })();
        if let Err(error)=result {state.failed+=1;if state.errors.len()<50{state.errors.push(format!("{relative}: {error}"));}}
        state.completed+=1;progress(state.clone());
    }
    if cancel.load(Ordering::Relaxed){return Err(invalid("Scan cancelled"));}
    catalog.lock().map_err(|_|invalid("Catalog unavailable"))?.reconcile(&source,&seen,complete)?;
    state.status=if state.failed>0||!complete{"completed with errors"}else{"complete"}.into();state.current.clear();progress(state.clone());Ok(state)
}
pub fn supported(path:&Path)->bool {path.extension().and_then(|s|s.to_str()).is_some_and(|s|["wav","mp3","flac","ogg","opus","m4a","aac","aif","aiff","wma","caf"].contains(&s.to_lowercase().as_str()))}
