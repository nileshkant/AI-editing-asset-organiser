use crate::{catalog::Profile, invalid, Result};
use serde::Deserialize;
use std::{io::{Read, BufReader}, path::{Path,PathBuf}, process::{Command,Stdio}, sync::{Arc,Mutex,atomic::{AtomicBool,Ordering}}, thread, time::{Duration,Instant}};

#[derive(Clone)]
pub struct MediaTools { pub ffmpeg: PathBuf, pub ffprobe: PathBuf }

impl MediaTools {
    pub fn validate(&self) -> Result<()> {
        if !self.ffmpeg.is_file() || !self.ffprobe.is_file() {return Err(invalid("FFmpeg media tools are unavailable"));}Ok(())
    }
}
#[derive(Deserialize)]
struct Probe { streams: Vec<Stream> }
#[derive(Deserialize)]
struct Stream { sample_rate: String, channels: u16, duration: Option<String> }

// Watchdog owns process cleanup even when pipe reads block on a broken decoder.
pub fn run_stream<T>(mut command: Command, cancel: Arc<AtomicBool>, timeout: Duration, read: impl FnOnce(&mut dyn Read)->Result<T>) -> Result<T> {
    let mut process=command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let mut stdout=BufReader::new(process.stdout.take().ok_or_else(||invalid("Decoder has no output"))?);
    let process=Arc::new(Mutex::new(process));let monitor=process.clone();
    let finished=Arc::new(AtomicBool::new(false));let stop=finished.clone();let aborted=Arc::new(AtomicBool::new(false));let abort=aborted.clone();
    let watchdog=thread::spawn(move || {let start=Instant::now();while !stop.load(Ordering::Relaxed) {if cancel.load(Ordering::Relaxed)||start.elapsed()>timeout {abort.store(true,Ordering::Relaxed);if let Ok(mut p)=monitor.lock(){let _=p.kill();}break;}thread::sleep(Duration::from_millis(50));}});
    let result=read(&mut stdout);
    if result.is_err() {if let Ok(mut p)=process.lock(){let _=p.kill();}}
    let status=loop {match process.lock().map_err(|_|invalid("Decoder state failed"))?.try_wait() {Ok(Some(status))=>break Ok(status),Err(error)=>break Err(error),Ok(None)=>{}}thread::sleep(Duration::from_millis(10));};
    finished.store(true,Ordering::Relaxed);let _=watchdog.join();
    if aborted.load(Ordering::Relaxed){return Err(invalid("Media operation cancelled or timed out"));}
    if !status?.success(){return Err(invalid("Media decoder failed; file may be corrupt or unsupported"));}
    result
}

pub fn analyze(tools:&MediaTools,path:&Path,cancel:Arc<AtomicBool>) -> Result<Profile> {
    tools.validate()?;
    let mut probe=Command::new(&tools.ffprobe);
    probe.args(["-v","error","-select_streams","a:0","-show_entries","stream=sample_rate,channels,duration","-of","json"]).arg(path);
    let bytes=run_stream(probe,cancel.clone(),Duration::from_secs(30),|r| {let mut b=Vec::new();r.take(1_048_577).read_to_end(&mut b)?;if b.len()>1_048_576{return Err(invalid("Metadata exceeds limit"));}Ok(b)})?;
    let probe:Probe=serde_json::from_slice(&bytes)?;
    let stream=probe.streams.first().ok_or_else(||invalid("No audio stream"))?;
    let rate:u32=stream.sample_rate.parse().map_err(|_|invalid("Invalid sample rate"))?;
    if !(8000..=384000).contains(&rate)||stream.channels==0||stream.channels>32{return Err(invalid("Unsupported sample rate or channel count"));}
    let expected=stream.duration.as_ref().and_then(|s|s.parse::<f64>().ok()).filter(|s|s.is_finite()&&*s>0.0).unwrap_or(60.0);
    let bucket=((expected*rate as f64/1600.0).ceil() as usize).max(1);
    let mut decode=Command::new(&tools.ffmpeg);
    decode.args(["-v","error","-nostdin","-threads","1","-i"]).arg(path).args(["-map","0:a:0","-vn","-f","f32le","-acodec","pcm_f32le","pipe:1"]);
    let channels=stream.channels;
    let mut profile=run_stream(decode,cancel,Duration::from_secs(7200),|r| measure_pcm(r,rate,channels,bucket))?;
    let texture=if profile.rms<0.01 {"quiet"} else if profile.rms>0.2 {"loud"} else {"moderate level"};
    let envelope=if profile.peak/(profile.rms.max(0.00001))>6.0 {"pronounced peaks"} else {"a relatively even level"};
    profile.description=format!("A {:.2}-second recording with {texture} average energy and {envelope}. Measured across the full recording; the event itself has not been identified by listening.",profile.duration);
    profile.tags=vec![texture.into(),if profile.duration<3.0 {"short"}else if profile.duration>30.0 {"long"}else{"medium duration"}.into(),if channels==1{"mono"}else{"multichannel"}.into()];
    if profile.peak<0.0001 {profile.tags.push("near silence".into());}
    Ok(profile)
}

pub fn measure_pcm(reader:&mut dyn Read,rate:u32,channels:u16,initial_bucket:usize)->Result<Profile>{
    if rate==0||channels==0{return Err(invalid("Invalid PCM format"));}
    let frame_bytes=channels as usize*4;let mut bytes=vec![0u8;frame_bytes];let mut samples=0u64;let mut sum=0.0f64;let mut peak=0.0f32;let mut frames=0u64;
    let mut waveform:Vec<[f32;2]>=vec![];let mut bucket=initial_bucket.max(1);let mut count=0usize;let mut low=f32::INFINITY;let mut high=f32::NEG_INFINITY;
    loop {
        let mut have=0;while have<frame_bytes {let n=reader.read(&mut bytes[have..])?;if n==0{break;}have+=n;}
        if have==0{break;}if have!=frame_bytes{return Err(invalid("Truncated decoded PCM"));}
        for raw in bytes.chunks_exact(4){let sample=f32::from_le_bytes(raw.try_into().unwrap());if !sample.is_finite(){return Err(invalid("Non-finite decoded samples"));}peak=peak.max(sample.abs());sum+=(sample as f64).powi(2);samples+=1;low=low.min(sample);high=high.max(sample);}
        frames+=1;count+=1;
        if count==bucket {waveform.push([low,high]);count=0;low=f32::INFINITY;high=f32::NEG_INFINITY;
            if waveform.len()>=3200 {waveform=waveform.chunks_exact(2).map(|p|[p[0][0].min(p[1][0]),p[0][1].max(p[1][1])]).collect();bucket*=2;}
        }
    }
    if samples==0{return Err(invalid("Audio has no decoded frames"));}
    if count>0{waveform.push([low,high]);}
    Ok(Profile{duration:frames as f64/rate as f64,sample_rate:rate,channels,frames,peak,rms:(sum/samples as f64).sqrt() as f32,description:String::new(),tags:vec![],waveform})
}

#[cfg(test)]mod tests{
 use super::*;
 #[test]fn stereo_does_not_cancel(){let data:Vec<u8>=[0.5f32,-0.5,0.5,-0.5].into_iter().flat_map(f32::to_le_bytes).collect();let p=measure_pcm(&mut &data[..],48000,2,1).unwrap();assert_eq!(p.rms,0.5);assert_eq!(p.peak,0.5);assert_eq!(p.frames,2);assert_eq!(p.waveform[0],[-0.5,0.5]);}
 #[test]fn nonfinite_rejected(){assert!(measure_pcm(&mut &f32::NAN.to_le_bytes()[..],48000,1,1).is_err());}
 #[test]fn truncated_rejected(){assert!(measure_pcm(&mut &[0u8;3][..],48000,1,1).is_err());}
 #[test]fn waveform_bounded(){let data:Vec<u8>=(0..100000).flat_map(|_|0.2f32.to_le_bytes()).collect();let p=measure_pcm(&mut &data[..],48000,1,1).unwrap();assert!(p.waveform.len()<3200);assert_eq!(p.frames,100000);}
 #[test]fn silence_valid(){let p=measure_pcm(&mut &[0u8;16][..],48000,1,2).unwrap();assert_eq!(p.rms,0.0);assert_eq!(p.waveform,vec![[0.0,0.0];2]);}
}
