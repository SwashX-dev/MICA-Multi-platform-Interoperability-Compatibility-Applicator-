//! Print what `mica-probe` learns about this machine, as JSON.

fn main() {
    let started = std::time::Instant::now();
    let profile = mica_probe::probe();
    println!("{}", serde_json::to_string_pretty(&profile).unwrap());
    eprintln!("probed in {:?}", started.elapsed());
}
