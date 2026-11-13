//! Times a full history load and reports peak memory:
//! `cargo run --release -p mugetsu-git --example load_bench -- <repo>`.

use std::time::Instant;

use mugetsu_git::{CommitOrder, Repo};

fn peak_rss_mb() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc_rusage>::zeroed();
    unsafe { getrusage(0, usage.as_mut_ptr()) };
    let kb = unsafe { usage.assume_init() }.ru_maxrss as f64;
    if cfg!(target_os = "macos") {
        kb / 1048576.
    } else {
        kb / 1024.
    }
}

#[repr(C)]
struct libc_rusage {
    ru_utime: [i64; 2],
    ru_stime: [i64; 2],
    ru_maxrss: i64,
    _rest: [i64; 13],
}

unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut libc_rusage) -> i32;
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let repo = Repo::open(path)?;
    let start = Instant::now();
    let refs = repo.refs()?;
    let history = repo.history(&refs.tips(), CommitOrder::Topological)?;
    let folded = mugetsu_git::Folded::new(&history, &Default::default(), &Default::default());
    println!(
        "{} commits, {} rows shown, {:?}, peak RSS {:.0} MB",
        history.len(),
        folded.rows.len(),
        start.elapsed(),
        peak_rss_mb()
    );
    let strings: usize = history
        .commits
        .iter()
        .map(|c| c.author.capacity() + c.email.capacity() + c.summary.capacity())
        .sum();
    let mut uniq = std::collections::HashSet::new();
    for c in &history.commits {
        uniq.insert((&c.author, &c.email));
    }
    println!(
        "Commit {} B, strings {:.1} MB, {} distinct authors",
        std::mem::size_of::<mugetsu_git::Commit>(),
        strings as f64 / 1e6,
        uniq.len()
    );
    Ok(())
}
