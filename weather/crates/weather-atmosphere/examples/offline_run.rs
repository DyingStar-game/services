//! Offline run of the atmosphere model on one body: spin-up, then a recorded period with PNG
//! tables, climate records, restart files and the guard (`run::run_pe`). For R&D and for making a
//! "warm" state ahead of time; the service does not need it.
//!
//! From `services/weather`:
//!   cargo run --release -p weather-atmosphere --example offline_run -- --body Sandbox --out out/sandbox \
//!       --spinup-days 5 --record-days 10
//! Options: --body NAME, --settings FILE (default: data/atmosphere/<body>.toml or defaut.toml),
//! --system FILE, --extras FILE, --set key=value (repeatable), --out DIR, --spinup-days N,
//! --record-days N, --restart FILE, --threads N (at most 4), --show (print the resolved values).

use std::path::PathBuf;
use weather_atmosphere::body::DataFiles;
use weather_atmosphere::settings::Settings;
use weather_atmosphere::setup::Scenario;

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let weather = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut body = "Sandbox".to_string();
    let mut settings: Option<PathBuf> = None;
    let mut system = weather.join("../resourcesDynamic/data/system/tarsis.json");
    let mut extras = weather.join("data/climate/tarsis.json");
    let mut sets: Vec<(String, String)> = Vec::new();
    let mut threads = 4usize;
    let mut show = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--body" => body = val()?,
            "--settings" => settings = Some(PathBuf::from(val()?)),
            "--system" => system = PathBuf::from(val()?),
            "--extras" => extras = PathBuf::from(val()?),
            "--set" => {
                let kv = val()?;
                let (k, v) = kv.split_once('=').ok_or_else(|| format!("--set {kv}: expected key=value"))?;
                sets.push((k.to_string(), v.to_string()));
            }
            "--out" => sets.push(("run.out_dir".into(), format!("'{}'", val()?))),
            "--restart" => sets.push(("run.restart_from".into(), format!("'{}'", val()?))),
            "--spinup-days" => sets.push(("run.spinup_days".into(), val()?)),
            "--record-days" => sets.push(("run.record_days".into(), val()?)),
            "--threads" => threads = val()?.parse().map_err(|e| format!("--threads: {e}"))?,
            "--show" => show = true,
            _ => return Err(format!("unknown option {a} (see the header of examples/offline_run.rs)")),
        }
    }
    let files = DataFiles { system, extras: Some(extras) };
    let path = settings.unwrap_or_else(|| Settings::file_for_body(&weather.join("data/atmosphere"), &body));
    let sc = Scenario::load_with_data(&path, &sets, Some(&body), &files)?;
    println!("{}", sc.summary());
    if show {
        return Ok(());
    }
    if sc.s.run.out_dir.is_none() {
        return Err("give an output folder with --out DIR".into());
    }
    let r = weather_atmosphere::with_threads(threads, || weather_atmosphere::run::run_pe(&sc))?;
    println!(
        "{} steps in {:.0} s; air drift {:+.1e}, max wind {:.1} m/s, ps {:.0}..{:.0} hPa; guard events: {}",
        r.steps,
        r.wall_s,
        r.air_drift,
        r.max_speed,
        r.min_ps / 100.0,
        r.max_ps / 100.0,
        r.guard_events.len()
    );
    if let Some(f) = r.failed {
        return Err(f);
    }
    Ok(())
}
