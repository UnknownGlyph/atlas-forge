//! Native project I/O with a distinct, reproducible binary attachment per board.
use map_core::{underlay::Blob, world_book::{Attachment, Link, Node, WorldBook}, Document};
use std::{sync::Arc, time::Instant};

pub(super) fn run(count: usize, kib: usize) {
    let mut doc = Document::default();
    let mut state = 2897u32;
    let book = WorldBook::edit(&mut doc);
    for i in 0..count {
        let board = book.add_map(Document::new(&format!("Board {i}"), 6371.0));
        let mut bytes = vec![0; kib.checked_mul(1024).expect("attachment size overflow")];
        for chunk in bytes.chunks_exact_mut(4) {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            chunk.copy_from_slice(&state.to_le_bytes());
        }
        let asset = book.add_asset(Attachment { name: format!("{i}.bin"), bytes: Blob(Arc::new(bytes)) });
        book.add_link(Link::new(Node::Map(board), Node::Asset(asset), false)).unwrap();
    }
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("boards.atlas");
    let mut writes = Vec::new();
    let mut reads = Vec::new();
    for trial in 0..6 {
        doc.meta.name = format!("Revision {trial}");
        let start = Instant::now();
        map_io::project::save(&path, &doc).unwrap();
        let saved = start.elapsed();
        let start = Instant::now();
        let restored = map_io::project::load(&path).unwrap();
        let loaded = start.elapsed();
        let before = doc.book.as_ref().unwrap();
        let after = restored.book.as_ref().unwrap();
        assert_eq!(restored.meta.name, doc.meta.name);
        assert_eq!(after.boards.len(), count + 1);
        assert_eq!(after.links, before.links);
        assert_eq!(after.assets, before.assets);
        if trial > 0 {
            writes.push(saved.as_secs_f64() * 1000.0);
            reads.push(loaded.as_secs_f64() * 1000.0);
        }
    }
    writes.sort_by(f64::total_cmp);
    reads.sort_by(f64::total_cmp);
    println!("child_boards={count} attachment_kib={kib} file_bytes={} median_save_ms={:.2} median_load_ms={:.2}",
        path.metadata().unwrap().len(), writes[2], reads[2]);
}
