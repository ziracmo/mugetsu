use std::time::Instant;
fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let repo = mugetsu_git::Repo::open(&path)?;
    let tips = repo.refs()?.tips();
    let t = Instant::now();
    let mut g = gix::open(&path)?;
    println!("open {:?}", t.elapsed());
    g.object_cache_size_if_unset(64 << 20);
    let t = Instant::now();
    let tips: Vec<_> = tips.iter().filter_map(|id| g.find_object(*id).ok()?.peel_to_commit().ok().map(|c| c.id)).collect();
    println!("peel {:?}", t.elapsed());
    let t = Instant::now();
    let ids: Vec<_> = g.rev_walk(tips).all()?.map(|i| i.unwrap().id).collect();
    println!("walk bfs {} {:?}", ids.len(), t.elapsed());
    let t = Instant::now();
    for id in &ids { let c = g.find_commit(*id)?; let d = c.decode()?; std::hint::black_box(d.message_summary()); }
    println!("decode serial {:?}", t.elapsed());
    Ok(())
}
