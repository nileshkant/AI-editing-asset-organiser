use std::{io::Write, path::Path};
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        return Err(
            "Usage: creativeshelf-handoff <export.sidecar.json> <project/public> <new-props.json>"
                .into(),
        );
    }
    let props =
        soundshelf_core::handoff::remotion_handoff(Path::new(&args[0]), Path::new(&args[1]))?;
    let bytes = serde_json::to_vec_pretty(&props)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[2])?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    println!("Portable Remotion audio props written");
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
