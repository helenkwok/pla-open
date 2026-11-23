use std::process::ExitCode;

use pla_open::{Pla, Result};
use serde_json::json;

const USAGE: &str = "usage: pla-open info FILE\n       pla-open stories FILE [--json]\n       pla-open walls FILE [--json]";

fn info(path: &str) -> Result<()> {
    let mut pla = Pla::open(path)?;
    let mut failed = 0usize;
    let stats = pla.walk(|b| {
        if b.info.block_type != pla_open::FREE_BLOCK && b.payload().is_err() {
            failed += 1;
        }
        Ok(())
    })?;
    println!("file size: {} bytes", pla.size());
    let words: Vec<String> = pla
        .header_words
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!("header words: {}", words.join(" "));
    println!(
        "blocks: {} (one-unit: {})  compressed sections: {}  gap units: {}  payloads not decoded: {}",
        stats.blocks, stats.short_blocks, stats.sections, stats.gap_units, failed
    );
    println!("blocks by type:");
    for (ty, n) in &stats.by_type {
        println!("  {ty:>6}  {n}");
    }
    Ok(())
}

fn walls(path: &str, as_json: bool) -> Result<()> {
    let walls = Pla::open(path)?.walls()?;
    if as_json {
        let v: Vec<_> = walls
            .iter()
            .map(|w| {
                json!({
                    "guid": w.guid.to_string(),
                    "id": w.id,
                    "begin": {"x": w.begin[0], "y": w.begin[1]},
                    "end": {"x": w.end[0], "y": w.end[1]},
                    "beginThickness": w.begin_thickness,
                    "endThickness": w.end_thickness,
                    "height": w.height,
                    "bottomOffset": w.bottom_offset,
                    "topOffset": w.top_offset,
                    "topLinked": w.top_linked,
                    "homeStory": w.home_story.map(|g| g.to_string()),
                    "homeStoryIndex": w.home_story_index,
                    "homeStoryName": w.home_story_name,
                    "topStoryIndex": w.top_story_index,
                    "relativeTopStory": w.relative_top_story,
                    "referenceLine": w.reference_line.to_string(),
                    "referenceLineOffset": w.reference_line_offset,
                    "offsetFromOutside": w.offset_from_outside,
                    "flipped": w.flipped,
                    "structure": w.structure.to_string(),
                    "compositeIndex": w.composite_index,
                    "compositeName": w.composite_name,
                    "compositeGuid": w.composite_guid.map(|g| g.to_string()),
                    "geometry": w.geometry.to_string(),
                    "arcAngle": w.arc_angle,
                    "profile": w.profile.to_string(),
                    "slantAlpha": w.slant_alpha,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&v).expect("JSON"));
    } else {
        for w in &walls {
            let h = w.height.map_or("?".to_string(), |h| h.to_string());
            println!(
                "{} {:?} ({}, {}) -> ({}, {}) thickness {}/{} height {} story {}",
                w.guid,
                w.id,
                w.begin[0],
                w.begin[1],
                w.end[0],
                w.end[1],
                w.begin_thickness,
                w.end_thickness,
                h,
                w.home_story_name.as_deref().unwrap_or("?")
            );
        }
    }
    Ok(())
}

fn stories(path: &str, as_json: bool) -> Result<()> {
    let stories = Pla::open(path)?.stories()?;
    if as_json {
        let v: Vec<_> = stories
            .iter()
            .map(|s| {
                json!({
                    "index": s.index,
                    "level": s.level,
                    "height": s.height,
                    "name": s.name,
                    "virtual": s.is_virtual,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&v).expect("JSON"));
    } else {
        for s in &stories {
            let h = s.height.map_or("?".to_string(), |h| h.to_string());
            let v = if s.is_virtual { " (virtual)" } else { "" };
            println!("{} level {} height {} {:?}{v}", s.index, s.level, h, s.name);
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["info", f] => info(f),
        ["stories", f] => stories(f, false),
        ["stories", f, "--json"] | ["stories", "--json", f] => stories(f, true),
        ["walls", f] => walls(f, false),
        ["walls", f, "--json"] | ["walls", "--json", f] => walls(f, true),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pla-open: {e}");
            ExitCode::FAILURE
        }
    }
}
