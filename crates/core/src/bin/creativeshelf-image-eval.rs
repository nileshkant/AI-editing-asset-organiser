//! SS-050 image-AI evaluation tool. Development use only; not shipped.
//!
//!   creativeshelf-image-eval corpus <dir>
//!   creativeshelf-image-eval baseline <dir>
//!   creativeshelf-image-eval run <dir> --thresholds <file> --max-requests <n> --out <report.json>
//!       --kind <openai|anthropic|gemini|openai_compatible> --model <id> [--base-url <url>] [--allow-local] [--key-env <VAR>]
//!       [--embed-kind <kind> --embed-model <id> [--embed-base-url <url>] [--embed-key-env <VAR>]]
//!
//! `run` contacts only the providers named on the command line, sends only the
//! images listed in `<dir>/manifest.json` whose digests match, and stops at
//! `--max-requests`. Keys are read from the named environment variables and
//! never printed.
use soundshelf_core::ai_provider::{Capabilities, KeyStorage, ProviderKind, ProviderProfile, Secret};
use soundshelf_core::ai_provider_wire::HttpAdapter;
use soundshelf_core::image_eval::{self, Describer, Embedder, Manifest, RunInput, Thresholds};
use std::collections::HashMap;
use std::path::Path;

type Error = Box<dyn std::error::Error>;

fn flags(args: &[String]) -> Result<HashMap<String, String>, Error> {
    let mut out = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let name = args[i].strip_prefix("--").ok_or_else(|| format!("unexpected argument {}", args[i]))?;
        if name == "allow-local" {
            out.insert(name.to_owned(), "true".to_owned());
            i += 1;
            continue;
        }
        let value = args.get(i + 1).ok_or_else(|| format!("--{name} needs a value"))?;
        out.insert(name.to_owned(), value.clone());
        i += 2;
    }
    Ok(out)
}

fn adapter(f: &HashMap<String, String>, prefix: &str) -> Result<Option<(HttpAdapter, ProviderKind, String)>, Error> {
    let key = |n: &str| format!("{prefix}{n}");
    let Some(kind) = f.get(&key("kind")) else { return Ok(None) };
    let kind: ProviderKind = serde_json::from_value(serde_json::Value::String(kind.clone())).map_err(|_| format!("unknown provider kind {kind}"))?;
    let model = f.get(&key("model")).ok_or_else(|| format!("--{} is required", key("model")))?.clone();
    let base_url = match f.get(&key("base-url")) {
        Some(u) => u.clone(),
        None => kind.default_base_url().ok_or("--base-url is required for this provider")?.to_owned(),
    };
    let secret = match f.get(&key("key-env")) {
        Some(var) => Some(Secret::new(&std::env::var(var).map_err(|_| format!("environment variable {var} is not set"))?)?),
        None => None,
    };
    let profile = ProviderProfile {
        id: format!("evaluation-{prefix}provider"),
        kind,
        label: "Evaluation".into(),
        base_url,
        model: model.clone(),
        allow_local: f.contains_key("allow-local"),
        key_storage: if secret.is_some() { KeyStorage::Session } else { KeyStorage::None },
        key_origin: None,
        revision: 1,
        capabilities: Capabilities::default(),
    };
    Ok(Some((HttpAdapter::new(&profile, secret)?, kind, model)))
}

fn run() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, dir) = match (args.first(), args.get(1)) {
        (Some(c), Some(d)) => (c.as_str(), Path::new(d)),
        _ => return Err("usage: creativeshelf-image-eval <corpus|baseline|run> <dir> [options]".into()),
    };
    match command {
        "corpus" => {
            let m = image_eval::generate_corpus(dir)?;
            println!("Wrote {} images and manifest.json to {}", m.items.len(), dir.display());
        }
        "baseline" => {
            let m = Manifest::load(&dir.join("manifest.json"))?;
            let ranks = image_eval::filename_baseline(&m);
            for split in [image_eval::Split::Dev, image_eval::Split::Heldout] {
                let pool = m.items.iter().filter(|i| i.split == split).map(|i| i.id.clone()).collect();
                let s = image_eval::score_retrieval(&m.queries, &ranks, &pool);
                println!("{split:?}: {}", serde_json::to_string(&s)?);
            }
        }
        "run" => {
            let f = flags(&args[2..])?;
            let manifest_path = dir.join("manifest.json");
            let manifest = Manifest::load(&manifest_path)?;
            let thresholds_path = f.get("thresholds").ok_or("--thresholds is required")?;
            let thresholds_bytes = std::fs::read(thresholds_path)?;
            let thresholds: Thresholds = serde_json::from_slice(&thresholds_bytes)?;
            let max: u32 = f.get("max-requests").ok_or("--max-requests is required")?.parse()?;
            let out = f.get("out").ok_or("--out is required")?;
            let describer = adapter(&f, "")?.ok_or("--kind is required")?;
            let embedder = adapter(&f, "embed-")?;
            let report = image_eval::run(
                &RunInput {
                    dir,
                    manifest: &manifest,
                    manifest_blake3: blake3::hash(&std::fs::read(&manifest_path)?).to_hex().to_string(),
                    thresholds: &thresholds,
                    thresholds_blake3: blake3::hash(&thresholds_bytes).to_hex().to_string(),
                    request_allowance: max,
                },
                Some(&Describer { adapter: &describer.0, kind: describer.1, model: describer.2.clone() }),
                embedder.as_ref().map(|e| Embedder { adapter: &e.0, kind: e.1, model: e.2.clone() }).as_ref(),
            )?;
            std::fs::write(out, serde_json::to_string_pretty(&report)? + "\n")?;
            println!("{} requests; verdict: {}", report.requests, serde_json::to_string(&report.verdict)?);
        }
        _ => return Err(format!("unknown command {command}").into()),
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
