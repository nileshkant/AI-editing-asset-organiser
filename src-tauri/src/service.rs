use soundshelf_core::{
    catalog::{Catalog, Source},
    jobs::{now_secs, Job},
    library::{scan, Progress},
    media::MediaTools,
    playback::Player,
    waveform::WaveformService,
};
use std::{path::PathBuf,sync::{Arc,Mutex,mpsc::{sync_channel,SyncSender,RecvTimeoutError},atomic::{AtomicBool,Ordering}},thread,time::Duration};
use uuid::Uuid;

struct Work { source:Source, job_id:String }

pub struct AppState {
    pub data_directory:PathBuf,
    pub catalog:Arc<Mutex<Catalog>>,
    pub tools:Option<MediaTools>,
    pub player:Arc<Player>,
    pub progress:Arc<Mutex<Vec<Progress>>>,
    pub waveforms:Arc<WaveformService>,
    pub cancel:Arc<AtomicBool>,
    stop:Arc<AtomicBool>,
    sender:SyncSender<Work>,
    worker:Mutex<Option<thread::JoinHandle<()>>>,
}
impl AppState {
    pub fn new(data_directory:PathBuf,resources:PathBuf)->Result<Self,Box<dyn std::error::Error>>{
        std::fs::create_dir_all(&data_directory)?;
        let cache_dir = data_directory.join("cache").join("waveforms");
        std::fs::create_dir_all(&cache_dir)?;
        let waveforms = Arc::new(WaveformService::new(cache_dir));
        let catalog=Arc::new(Mutex::new(Catalog::open(&data_directory.join("library.sqlite"))?));
        let mut tools=MediaTools{ffmpeg:resources.join("media").join(if cfg!(windows){"ffmpeg.exe"}else{"ffmpeg"}),ffprobe:resources.join("media").join(if cfg!(windows){"ffprobe.exe"}else{"ffprobe"})};
        if tools.validate().is_err() {
            if let Some(discovered) = MediaTools::discover() {
                tools = discovered;
            }
        }
        let tools=tools.validate().ok().map(|_|tools);
        let progress=Arc::new(Mutex::new(Vec::<Progress>::new()));let cancel=Arc::new(AtomicBool::new(false));let stop=Arc::new(AtomicBool::new(false));
        let owner=Uuid::new_v4().to_string();
        let(tx,rx)=sync_channel::<Work>(32);let db=catalog.clone();let updates=progress.clone();let flag=cancel.clone();let quit=stop.clone();let media=tools.clone();let worker_owner=owner.clone();
        let worker=thread::spawn(move||loop {
            if quit.load(Ordering::Relaxed){break;}
            let work=match rx.recv_timeout(Duration::from_millis(100)){Ok(s)=>s,Err(RecvTimeoutError::Timeout)=>continue,Err(_)=>break};
            flag.store(false,Ordering::Relaxed);let id=work.source.id.clone();let job_id=work.job_id.clone();
            let claimed={
                match db.lock() {
                    Ok(catalog)=>catalog.claim_job(&job_id,&worker_owner,now_secs()).ok().flatten(),
                    Err(_)=>None,
                }
            };
            if claimed.is_none(){continue;}
            if let Some(tools)=&media {
                let persist=db.clone();let persist_owner=worker_owner.clone();
                let result=scan(db.clone(),work.source,tools,flag.clone(),job_id.clone(),|p|{
                    if let Ok(catalog)=persist.lock(){let _=catalog.persist_progress(&persist_owner,&p);}
                    if let Ok(mut list)=updates.lock(){if let Some(old)=list.iter_mut().find(|s|s.source_id==p.source_id){*old=p;}else{list.push(p);}}
                });
                match result {
                    Ok(p)=>{if let Ok(catalog)=db.lock(){let _=catalog.finish_job(&job_id,&worker_owner,&p,false);} if let Ok(mut list)=updates.lock(){if let Some(old)=list.iter_mut().find(|s|s.source_id==p.source_id){*old=p;}else{list.push(p);}}}
                    Err(error)=>{
                        let cancelled=flag.load(Ordering::Relaxed);
                        let mut failed=Progress{job_id:job_id.clone(),source_id:id.clone(),status:if cancelled{"cancelled"}else{"failed"}.into(),errors:vec![error.to_string()],..Default::default()};
                        if let Ok(list)=updates.lock(){if let Some(p)=list.iter().find(|s|s.source_id==id){failed.completed=p.completed;failed.total=p.total;failed.reused=p.reused;failed.failed=p.failed;failed.current=p.current.clone();failed.errors.extend(p.errors.clone());}}
                        if let Ok(catalog)=db.lock(){let _=catalog.finish_job(&job_id,&worker_owner,&failed,cancelled);}
                        if let Ok(mut list)=updates.lock(){if let Some(p)=list.iter_mut().find(|s|s.source_id==id){*p=failed;}}
                    }
                }
            }
        });
        let player=Arc::new(Player::new(tools.clone()));
        let state=Self{data_directory,catalog,tools,player,progress,waveforms,cancel,stop,sender:tx,worker:Mutex::new(Some(worker))};
        state.resume_persisted()?;
        Ok(state)
    }
    fn remember(&self, job:&Job)->Result<(),String>{
        let mut list=self.progress.lock().map_err(|e|e.to_string())?;
        list.retain(|p|p.source_id!=job.source_id);
        list.push(job.progress());
        Ok(())
    }
    fn resume_persisted(&self)->Result<(),String>{
        let jobs={
            let catalog=self.catalog.lock().map_err(|e|e.to_string())?;
            catalog.recover_jobs(now_secs()).map_err(|e|e.to_string())?
        };
        for job in jobs {
            self.remember(&job)?;
            let source=self.catalog.lock().map_err(|e|e.to_string())?.source(&job.source_id).map_err(|e|e.to_string())?;
            if self.sender.try_send(Work{source,job_id:job.id.clone()}).is_err() {
                return Err("Import queue is full".into());
            }
        }
        let history={
            let catalog=self.catalog.lock().map_err(|e|e.to_string())?;
            catalog.active_jobs().map_err(|e|e.to_string())?
        };
        let mut list=self.progress.lock().map_err(|e|e.to_string())?;
        for job in history {
            if !list.iter().any(|p|p.job_id==job.id){list.push(job.progress());}
        }
        Ok(())
    }
    pub fn enqueue(&self,source:Source)->Result<(),String>{
        if self.tools.is_none(){return Err("Media binaries are not included in this development package".into());}
        let job={
            let catalog=self.catalog.lock().map_err(|e|e.to_string())?;
            catalog.enqueue_scan(&source.id).map_err(|e|e.to_string())?
        };
        if job.state=="running" {
            self.remember(&job)?;
            return Ok(());
        }
        self.remember(&job)?;
        if self.sender.try_send(Work{source,job_id:job.id.clone()}).is_err() {
            return Err("Import queue is full".into());
        }
        Ok(())
    }
    pub fn shutdown(&self){
        self.player.stop();
        self.stop.store(true,Ordering::Relaxed);
        self.cancel.store(true,Ordering::Relaxed);
        if let Ok(catalog)=self.catalog.lock(){let _=catalog.checkpoint_running();}
        if let Ok(mut handle)=self.worker.lock(){if let Some(worker)=handle.take(){let _=worker.join();}}
    }
}
