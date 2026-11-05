//! Offline test of the packaged engine. Never records, injects text, or saves history.
pub fn run_if_requested() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("--self-test") {
        return false;
    }
    let result = (|| -> Result<(), String> {
        let path = args
            .get(2)
            .ok_or("Usage: gretchen-flow --self-test MANIFEST.json")?;
        let manifest: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let cfg = crate::config::Config {
            model: crate::config::DEFAULT_MODEL.into(),
            ..Default::default()
        };
        let model = crate::model::ensure_model(&cfg.model)?;
        let engine = crate::engine::Engine::load(&model.to_string_lossy(), &cfg)?;
        for clip in manifest {
            let path = clip["path"].as_str().ok_or("clip missing path")?;
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            if bytes.len() % 4 != 0 {
                return Err("invalid f32 PCM".into());
            }
            let samples: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|x| f32::from_le_bytes(*x))
                .collect();
            let start = std::time::Instant::now();
            let text = engine.transcribe(&samples, &cfg.for_app(None))?;
            println!(
                "{}",
                serde_json::json!({"id": clip["id"], "text": text, "seconds":start.elapsed().as_secs_f64()})
            );
        }
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
    true
}
