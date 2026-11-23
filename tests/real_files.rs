//! Checks walls and stories against `<name>.details.json` files next to real projects.
//!
//! Run with `PLA_OPEN_TEST_FILES=<folder>[:<folder>...] cargo test -- --ignored`.

use pla_open::{Guid, Pla, Profile, Structure, Wall};
use serde_json::Value;

fn close(a: f64, b: &Value) -> bool {
    b.as_f64().is_some_and(|b| (a - b).abs() < 1e-9)
}

/// Compare one wall with its details; returns the fields that differ.
/// Fields missing from the details are not compared.
fn compare_wall(w: &Wall, e: &Value) -> Vec<String> {
    let x = &e["details"];
    let mut bad = Vec::new();
    if e["id"].as_str() != Some(w.id.as_str()) {
        bad.push(format!("id {:?}", w.id));
    }
    let mut num = |label: &str, ours: Option<f64>, theirs: &Value| {
        if theirs.is_null() {
            return;
        }
        if !ours.is_some_and(|o| close(o, theirs)) {
            bad.push(format!("{label} {ours:?} vs {theirs}"));
        }
    };
    num("begin x", Some(w.begin[0]), &x["begCoordinate"]["x"]);
    num("begin y", Some(w.begin[1]), &x["begCoordinate"]["y"]);
    num("end x", Some(w.end[0]), &x["endCoordinate"]["x"]);
    num("end y", Some(w.end[1]), &x["endCoordinate"]["y"]);
    num(
        "begin thickness",
        Some(w.begin_thickness),
        &x["begThickness"],
    );
    num("end thickness", Some(w.end_thickness), &x["endThickness"]);
    num("bottom offset", Some(w.bottom_offset), &x["bottomOffset"]);
    num("top offset", Some(w.top_offset), &x["topOffset"]);
    num("height", w.height, &x["height"]);
    num("offset", Some(w.reference_line_offset), &x["offset"]);
    num(
        "offset from outside",
        Some(w.offset_from_outside),
        &x["offsetFromOutside"],
    );
    num("arc angle", Some(w.arc_angle), &x["arcAngle"]);
    num(
        "relative top story",
        w.relative_top_story.map(f64::from),
        &x["relativeTopStory"],
    );
    num(
        "home story",
        w.home_story_index.map(f64::from),
        &e["floorIndex"],
    );
    if w.profile == Profile::Slanted {
        num("slant alpha", w.slant_alpha, &x["slantAlpha"]);
    }

    let mut text = |label: &str, ours: String, theirs: &Value| {
        if let Some(t) = theirs.as_str() {
            if t != ours {
                bad.push(format!("{label} {ours} vs {t}"));
            }
        }
    };
    text("geometry", w.geometry.to_string(), &x["geometryType"]);
    text("structure", w.structure.to_string(), &x["structureType"]);
    text(
        "reference line",
        w.reference_line.to_string(),
        &x["referenceLineLocation"],
    );
    text("profile", w.profile.to_string(), &x["profileType"]);
    if let Some(f) = x["flipped"].as_bool() {
        if f != w.flipped {
            bad.push(format!("flipped {}", w.flipped));
        }
    }
    if w.structure == Structure::Composite {
        if let Some(c) = x["compositeId"]["guid"].as_str() {
            if w.composite_guid != Guid::parse(c) {
                bad.push(format!("composite {:?} vs {c}", w.composite_guid));
            }
        }
    }
    bad
}

#[test]
#[ignore = "needs PLA_OPEN_TEST_FILES"]
fn walls_and_stories_match_details() {
    let dirs = std::env::var_os("PLA_OPEN_TEST_FILES").expect("set PLA_OPEN_TEST_FILES");
    let mut files = 0;
    let mut matched = 0;
    let mut tables = 0;
    let mut problems = Vec::new();
    let mut entries: Vec<_> = std::env::split_paths(&dirs)
        .flat_map(|d| std::fs::read_dir(d).unwrap().map(|e| e.unwrap().path()))
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
        let stories = pla.stories().unwrap_or_else(|e| panic!("{name}: {e}"));
        println!(
            "{name}: {} blocks, {} sections, {} gap units, {} walls, {} stories",
            st.blocks,
            st.sections,
            st.gap_units,
            walls.len(),
            stories.len()
        );
        let details = path.with_file_name(format!("{name}.details.json"));
        let Ok(text) = std::fs::read_to_string(&details) else {
            continue;
        };
        files += 1;
        let d: Value = serde_json::from_str(&text).unwrap();

        if let Some(expected) = d["stories"]["stories"].as_array() {
            let real: Vec<_> = stories.iter().filter(|s| !s.is_virtual).collect();
            let same = real.len() == expected.len()
                && real.iter().zip(expected).all(|(s, x)| {
                    x["index"].as_u64() == Some(s.index.into())
                        && close(s.level, &x["level"])
                        && s.height.is_some_and(|h| close(h, &x["height"]))
                        && x["name"].as_str() == Some(s.name.as_str())
                });
            if same {
                tables += 1;
            } else {
                problems.push(format!("{name}: stories {real:?} vs {expected:?}"));
            }
        }

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
            // Match by GUID; older details files have none, so fall back to the element ID.
            let guid = e["guid"].as_str().and_then(Guid::parse);
            let label = e["guid"].as_str().or(e["id"].as_str()).unwrap_or("?");
            let found = walls.iter().find(|w| match guid {
                Some(g) => w.guid == g,
                None => e["id"].as_str() == Some(w.id.as_str()),
            });
            let Some(w) = found else {
                problems.push(format!("{name}: wall {label} not found"));
                continue;
            };
            let bad = compare_wall(w, e);
            if bad.is_empty() {
                matched += 1;
            } else {
                problems.push(format!("{name}: wall {label}: {}", bad.join(", ")));
            }
        }
    }
    println!("{matched} walls matched, {tables} story tables matched, in {files} files");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    assert!(files > 0, "no .pln files with .details.json found");
}
