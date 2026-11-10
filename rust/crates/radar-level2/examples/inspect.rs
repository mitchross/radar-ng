use anyhow::{Context, Result, ensure};
use radar_level2::decode::{Decoder, capped_read};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(args.len() == 3, "usage: inspect STATION FILE");
    let file = std::fs::File::open(&args[2])?;
    let bytes = capped_read(file, radar_level2::COMPRESSED_LIMIT)?;
    let mut decoder = Decoder::new(&args[1]);
    let sweeps = decoder.push(&bytes).context("bounded decoder")?;
    decoder.finish()?;
    serde_json::to_writer(std::io::stdout().lock(), &sweeps)?;
    Ok(())
}
