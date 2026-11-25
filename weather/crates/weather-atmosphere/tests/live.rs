//! The service pieces (`live`): rolling checkpoints in a `StateStore` (the newest healthy one is
//! picked, a damaged or unhealthy one is skipped, a resumed model continues bit for bit) and the
//! tables rendered in memory. Coarse grid (level 3) to stay short: the bookkeeping does not
//! depend on the grid.

mod common;

use weather_atmosphere::live::{CheckpointMeta, FileStateStore, LiveModel, RollingCheckpoints};
use weather_atmosphere::pe::ChunkOutcome;
use weather_atmosphere::setup::Scenario;
use weather_atmosphere::with_threads;

fn scenario() -> Scenario {
    let mut sc = Scenario::for_body(&common::settings_dir(), "Sandbox", &[], &common::files()).unwrap();
    sc.s.grid.level = 3;
    sc.s.run.img_w = 64;
    sc.s.run.img_h = 32;
    sc
}

fn meta(m: &LiveModel, healthy: bool) -> CheckpointMeta {
    CheckpointMeta {
        body: m.sc.body.name.clone(),
        chunk: m.st.chunk,
        model_time_s: m.st.time,
        written_unix: 0.0,
        healthy,
        spun_up: false,
        anchor_model_s: 0.0,
        anchor_unix: 0.0,
    }
}

fn steps(m: &mut LiveModel, n: usize) {
    for _ in 0..n {
        assert!(matches!(m.step_chunk().unwrap(), ChunkOutcome::Done(_)));
    }
}

fn rolling(dir: &std::path::Path) -> RollingCheckpoints {
    RollingCheckpoints::new(Box::new(FileStateStore::new(dir)), "Sandbox")
}

#[test]
fn rolling_checkpoints_and_resume() {
    let _g = common::heavy();
    let dir = std::env::temp_dir().join("weather_atmosphere_live_test");
    let _ = std::fs::remove_dir_all(&dir);
    with_threads(2, || {
        let mut m = LiveModel::cold_start(scenario()).unwrap();
        let mut ck = rolling(&dir);
        assert!(ck.latest_healthy(&m.model).0.is_none());
        steps(&mut m, 3);
        ck.save(&m.model, &m.st, &meta(&m, true)).unwrap();
        steps(&mut m, 3);
        ck.save(&m.model, &m.st, &meta(&m, true)).unwrap();
        let at6 = m.st.clone();
        steps(&mut m, 4);
        let at10 = m.st.clone();

        // A new process: the newest checkpoint (chunk 6) is picked, and the run goes on bit for bit.
        let mut ck2 = rolling(&dir);
        let (found, skipped) = ck2.latest_healthy(&m.model);
        let (st, meta6) = found.expect("a checkpoint");
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(meta6.chunk, 6);
        assert!(st.bit_equal(&at6));
        let mut m2 = LiveModel::cold_start(scenario()).unwrap();
        m2.resume_from(st).unwrap();
        steps(&mut m2, 4);
        assert!(m2.st.bit_equal(&at10), "resumed model diverged");

        // The next save goes to the older slot (chunk 3), so the one we resumed from stays safe.
        ck2.save(&m2.model, &m2.st, &meta(&m2, true)).unwrap();
        let (found, _) = rolling(&dir).latest_healthy(&m2.model);
        assert_eq!(found.unwrap().1.chunk, 10);
        let (found, _) = rolling(&dir).latest_healthy(&m2.model);
        assert!(found.is_some());

        // Metadata that does not match its state is skipped: back to the other slot (chunk 6).
        let side = ["a", "b"].iter().map(|s| dir.join("sandbox").join(format!("checkpoint_{s}.toml"))).find(|p| std::fs::read_to_string(p).unwrap().contains("chunk = 10")).unwrap();
        let txt = std::fs::read_to_string(&side).unwrap();
        std::fs::write(&side, txt.replace("chunk = 10", "chunk = 99")).unwrap();
        let (found, skipped) = rolling(&dir).latest_healthy(&m2.model);
        assert_eq!(found.unwrap().1.chunk, 6);
        assert_eq!(skipped.len(), 1, "{skipped:?}");

        // A checkpoint saved as unhealthy is never loaded back.
        std::fs::write(&side, txt.replace("healthy = true", "healthy = false")).unwrap();
        let (found, skipped) = rolling(&dir).latest_healthy(&m2.model);
        assert_eq!(found.unwrap().1.chunk, 6);
        assert!(skipped.is_empty(), "{skipped:?}");
    });
}

#[test]
fn tables_in_memory() {
    let _g = common::heavy();
    with_threads(2, || {
        let mut m = LiveModel::cold_start(scenario()).unwrap();
        steps(&mut m, 1);
        let tables = m.tables().unwrap();
        let names: Vec<&str> = tables.iter().map(|t| t.0).collect();
        println!("{names:?}");
        for want in ["pmer", "ps", "u_bas", "v_bas", "t_bas", "poussiere", "t_sol", "soulevement"] {
            assert!(names.contains(&want), "missing table {want}");
        }
        let (_, t) = tables.iter().find(|t| t.0 == "t_bas").unwrap();
        assert!(t.min > 150.0 && t.max < 400.0 && t.min < t.max, "t_bas range {} .. {}", t.min, t.max);
        assert!(t.json.contains("\"field\": \"t_bas\""));
        let dec = png::Decoder::new(std::io::Cursor::new(&t.png));
        let rd = dec.read_info().unwrap();
        assert_eq!((rd.info().width, rd.info().height), (64, 32));
        assert_eq!(rd.info().bit_depth, png::BitDepth::Sixteen);
    });
}
