//! Checks walls against `<name>.details.json` files next to real projects.
//!
//! Run with `PLA_OPEN_TEST_FILES=<folder> cargo test -- --ignored`.

use pla_open::{Guid, Pla};
use serde_json::Value;

fn close(a: f64, b: &Value) -> bool {
    b.as_f64().is_some_and(|b| (a - b).abs() < 1e-9)
}

#[test]
#[ignore = "needs PLA_OPEN_TEST_FILES"]
fn walls_match_details() {
    let dir = std::env::var("PLA_OPEN_TEST_FILES").expect("set PLA_OPEN_TEST_FILES");
    let mut files = 0;
    let mut matched = 0;
    let mut problems = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "pln" | "pla") {
            continue;
        }
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let mut pla = Pla::open(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let st = pla
            .walk(|_| Ok(()))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let walls = pla.walls().unwrap_or_else(|e| panic!("{name}: {e}"));
        println!(
            "{name}: {} blocks, {} sections, {} gap units, {} walls",
            st.blocks,
            st.sections,
            st.gap_units,
            walls.len()
        );
        let details = path.with_file_name(format!("{name}.details.json"));
        let Ok(text) = std::fs::read_to_string(&details) else {
            continue;
        };
        files += 1;
        let d: Value = serde_json::from_str(&text).unwrap();
        let expected: Vec<&Value> = d["detailsOfElements"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "Wall")
            .collect();
        if expected.len() != walls.len() {
            problems.push(format!(
                "{name}: {} walls expected, {} read",
                expected.len(),
                walls.len()
            ));
        }
        for e in expected {
            let guid = Guid::parse(e["guid"].as_str().unwrap()).unwrap();
            let Some(w) = walls.iter().find(|w| w.guid == guid) else {
                problems.push(format!("{name}: wall {guid} not found"));
                continue;
            };
            let x = &e["details"];
            let mut bad = Vec::new();
            if e["id"].as_str() != Some(w.id.as_str()) {
                bad.push(format!("id {:?}", w.id));
            }
            let checks = [
                ("begin x", w.begin[0], &x["begCoordinate"]["x"]),
                ("begin y", w.begin[1], &x["begCoordinate"]["y"]),
                ("end x", w.end[0], &x["endCoordinate"]["x"]),
                ("end y", w.end[1], &x["endCoordinate"]["y"]),
                ("begin thickness", w.begin_thickness, &x["begThickness"]),
                ("end thickness", w.end_thickness, &x["endThickness"]),
                ("bottom offset", w.bottom_offset, &x["bottomOffset"]),
            ];
            for (label, ours, theirs) in checks {
                if !close(ours, theirs) {
                    bad.push(format!("{label} {ours} vs {theirs}"));
                }
            }
            // The stored height is only meaningful when the top is not linked.
            let linked = x["relativeTopStory"].as_i64().unwrap_or(0) != 0;
            match (linked, w.height) {
                (true, None) => {}
                (false, Some(h)) if close(h, &x["height"]) => {}
                _ => bad.push(format!(
                    "height {:?} (top linked {}) vs {} (relativeTopStory {})",
                    w.height, w.top_linked, x["height"], x["relativeTopStory"]
                )),
            }
            if bad.is_empty() {
                matched += 1;
            } else {
                problems.push(format!("{name}: wall {guid}: {}", bad.join(", ")));
            }
        }
    }
    println!("{matched} walls matched in {files} files");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(files > 0, "no .pln files with .details.json in {dir}");
}
