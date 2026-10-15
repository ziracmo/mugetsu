//! `cargo run --release --example bench -- /path/to/repo`
use std::time::Instant;

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let t = Instant::now();
    let repo = mugetsu_git::Repo::open(&path)?;
    let refs = repo.refs()?;
    println!(
        "refs      {:>6} tips   {:?}",
        refs.tips().len(),
        t.elapsed()
    );
    let t = Instant::now();
    let history = repo.history(&refs.tips(), mugetsu_git::CommitOrder::Date)?;
    println!("history   {:>6} commits {:?}", history.len(), t.elapsed());
    let t = Instant::now();
    let graph = mugetsu_git::layout(&history.commits);
    let width = graph.max_width;
    println!("layout    {:>6} lanes  {:?}", width, t.elapsed());
    let t = Instant::now();
    let segs: usize = (0..history.len().min(60))
        .map(|i| graph.row(i).upper.len())
        .sum();
    println!("60 rows   {segs:>6} segs   {:?}", t.elapsed());
    Ok(())
}
