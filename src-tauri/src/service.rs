use soundshelf_core::{catalog::{Catalog,Source},library::{scan,Progress},media::MediaTools};
use std::{path::PathBuf,sync::{Arc,Mutex,mpsc::{sync_channel,SyncSender,RecvTimeoutError},atomic::{AtomicBool,Ordering}},thread,time::Duration};

pub struct AppState {
    pub data_directory:PathBuf,
    pub catalog:Arc<Mutex<Catalog>>,
    pub tools:Option<MediaTools>,
    pub progress:Arc<Mutex<Vec<Progress>>>,
    pub cancel:Arc<AtomicBool>,
    stop:Arc<AtomicBool>,
    sender:SyncSender<Source>,
    worker:Mutex<Option<thread::JoinHandle<()>>>,
}
impl AppState {
    pub fn new(data_directory:PathBuf,resources:PathBuf)->Result<Self,Box<dyn std::error::Error>>{
        std::fs::create_dir_all(&data_directory)?;
        let catalog=Arc::new(Mutex::new(Catalog::open(&data_directory.join("library.sqlite"))?));
        let mut tools=MediaTools{ffmpeg:resources.join("media").join(if cfg!(windows){"ffmpeg.exe"}else{"ffmpeg"}),ffprobe:resources.join("media").join(if cfg!(windows){"ffprobe.exe"}else{"ffprobe"})};
        if cfg!(debug_assertions)&&tools.validate().is_err(){
            for base in ["/opt/homebrew/bin","/usr/local/bin","/usr/bin"] {let candidate=MediaTools{ffmpeg:PathBuf::from(base).join("ffmpeg"),ffprobe:PathBuf::from(base).join("ffprobe")};if candidate.validate().is_ok(){tools=candidate;break;}}
            if let (Some(a),Some(b))=(std::env::var_os("SOUNDSHELF_FFMPEG"),std::env::var_os("SOUNDSHELF_FFPROBE")){tools=MediaTools{ffmpeg:a.into(),ffprobe:b.into()};}
        }
        let tools=tools.validate().ok().map(|_|tools);
        let progress=Arc::new(Mutex::new(Vec::<Progress>::new()));let cancel=Arc::new(AtomicBool::new(false));let stop=Arc::new(AtomicBool::new(false));
        let(tx,rx)=sync_channel::<Source>(32);let db=catalog.clone();let updates=progress.clone();let flag=cancel.clone();let quit=stop.clone();let media=tools.clone();
        let worker=thread::spawn(move||loop {
            if quit.load(Ordering::Relaxed){break;}
            let source=match rx.recv_timeout(Duration::from_millis(100)){Ok(s)=>s,Err(RecvTimeoutError::Timeout)=>continue,Err(_)=>break};
            flag.store(false,Ordering::Relaxed);let id=source.id.clone();
            if let Some(tools)=&media {
                let result=scan(db.clone(),source,tools,flag.clone(),|p|{if let Ok(mut list)=updates.lock(){if let Some(old)=list.iter_mut().find(|s|s.source_id==p.source_id){*old=p;}else{list.push(p);}}});
                if let Err(error)=result {if let Ok(mut list)=updates.lock(){if let Some(p)=list.iter_mut().find(|s|s.source_id==id){p.status=if flag.load(Ordering::Relaxed){"cancelled"}else{"failed"}.into();p.errors.push(error.to_string());}}}
            }
        });
        Ok(Self{data_directory,catalog,tools,progress,cancel,stop,sender:tx,worker:Mutex::new(Some(worker))})
    }
    pub fn enqueue(&self,source:Source)->Result<(),String>{
        if self.tools.is_none(){return Err("Media binaries are not included in this development package".into());}
        {
            let mut list=self.progress.lock().map_err(|e|e.to_string())?;
            if list.iter().any(|p|p.source_id==source.id&&["queued","discovering","analyzing"].contains(&p.status.as_str())){return Ok(());}
            list.retain(|p|p.source_id!=source.id);
            list.push(Progress{source_id:source.id.clone(),status:"queued".into(),..Default::default()});
        }
        if self.sender.try_send(source.clone()).is_err() {
            if let Ok(mut list)=self.progress.lock(){list.retain(|p|p.source_id!=source.id||p.status!="queued");}
            return Err("Import queue is full".into());
        }
        Ok(())
    }
    pub fn shutdown(&self){self.stop.store(true,Ordering::Relaxed);self.cancel.store(true,Ordering::Relaxed);if let Ok(mut handle)=self.worker.lock(){if let Some(worker)=handle.take(){let _=worker.join();}}}
}
