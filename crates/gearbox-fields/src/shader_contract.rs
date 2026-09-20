//! What the shared cover rules require of the shaders that read them, checked
//! rather than remembered.
//!
//! `bare/shaders/cover.wgsl` exists because the grounds either side of a join
//! have to agree pixel for pixel, and held as separate copies they drift: a
//! sway noise that differed between two of them once put the ruts of the ground
//! and the ruts of the grass in different places, and two copies of the edge
//! wash left one side ragged and one ruled. Nothing in a compiler sees that,
//! and a broken WGSL import is *silent* — no diagnostic at any log level, just
//! a cover quietly behaving unlike the rest. So the parts of the contract that
//! can be read off the files are read off them here.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn shaders_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.wgsl` under the crate, as (path from `src`, contents).
fn shaders() -> Vec<(String, String)> {
    fn walk(at: &Path, root: &Path, into: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(at).expect("read shader directory") {
            let path = entry.expect("read shader entry").path();
            if path.is_dir() {
                walk(&path, root, into);
            } else if path.extension().is_some_and(|it| it == "wgsl") {
                let name = path.strip_prefix(root).expect("under src").display().to_string();
                into.push((name, std::fs::read_to_string(&path).expect("read shader")));
            }
        }
    }
    let root = shaders_dir();
    let mut found = Vec::new();
    walk(&root, &root, &mut found);
    found.sort();
    assert!(found.len() > 10, "found only {} shaders; the walk is wrong", found.len());
    found
}

const COVER: &str = "bare/shaders/cover.wgsl";

/// The rules whose whole purpose is that separate shaders answer alike: where a
/// way has worn, where two covers meet, which of two surfaces takes a pixel,
/// and the patches a ground thins its own grass by. A second copy of any of
/// these is the bug this file exists to catch.
const MUST_AGREE: [&str; 11] = [
    "worn", "washed_into", "height_blend", "settled", "verge_damp", "way_beyond", "taken",
    "lattice", "clump_frame", "edge_stray", "inside_field",
];

/// `pcg` and `rand` are not rules, only hashing, and a cover that does not wear
/// at all — the concrete yard — has no business importing the wear system for
/// them. They may be carried, but they may not *differ*.
const MAY_BE_CARRIED: [&str; 2] = ["pcg", "rand"];

/// The names a file declares, whether as `fn name(` or as a `const name:`.
fn declared(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line.strip_prefix("fn ").or_else(|| line.strip_prefix("const "))?;
            let name: String =
                rest.chars().take_while(|it| it.is_alphanumeric() || *it == '_').collect();
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

/// The names a file imports out of `cover.wgsl`.
fn imported_from_cover(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter(|line| line.contains("cover.wgsl\"::"))
        .filter_map(|line| line.split_once('{'))
        .filter_map(|(_, rest)| rest.split_once('}'))
        .flat_map(|(names, _)| names.split(',').map(|it| it.trim().to_owned()))
        .filter(|it| !it.is_empty())
        .collect()
}

/// Whether a name is *called* in a file, rather than merely imported. A call
/// site is the name followed by an open bracket, outside the import line.
fn calls(source: &str, name: &str) -> bool {
    source
        .lines()
        .filter(|line| !line.contains("cover.wgsl\"::"))
        .filter(|line| !line.trim_start().starts_with("//"))
        .any(|line| {
            line.match_indices(&format!("{name}(")).any(|(at, _)| {
                let before = line[..at].chars().next_back();
                // Not `fn worn(`, and not the tail of a longer identifier.
                !before.is_some_and(|it| it.is_alphanumeric() || it == '_')
                    && !line[..at].trim_end().ends_with("fn")
            })
        })
}

/// Declaring an import path makes the `embedded://` import resolve to nothing,
/// the importing shader silently draws nothing, and **no diagnostic appears at
/// any log level**. The working shared files in this crate declare none; that
/// is the whole convention, and it is one line to keep.
#[test]
fn no_shared_shader_declares_an_import_path() {
    for (name, source) in shaders() {
        assert!(
            !source.contains("#define_import_path"),
            "{name} declares an import path; every `embedded://` import of it \
             then resolves to nothing, silently"
        );
    }
}

/// A rule used from `cover.wgsl` must be imported from it. Copying the function
/// in instead is what the shared file exists to prevent, and a missing import
/// fails silently rather than loudly.
#[test]
fn every_cover_rule_is_imported_where_it_is_used() {
    let all = shaders();
    let cover = &all.iter().find(|(name, _)| name == COVER).expect("cover.wgsl").1;
    let rules = declared(cover);
    for shared in MUST_AGREE {
        assert!(rules.contains(shared), "cover.wgsl no longer declares `{shared}`");
    }
    for (name, source) in all.iter().filter(|(name, _)| name != COVER) {
        let imported = imported_from_cover(source);
        for rule in MUST_AGREE {
            if calls(source, rule) {
                assert!(
                    imported.contains(rule),
                    "{name} calls `{rule}` without importing it from cover.wgsl; \
                     either import it or it is a second copy, which drifts"
                );
            }
            assert!(
                !declared(source).contains(rule) || imported.contains(rule),
                "{name} declares its own `{rule}`; that is the second copy"
            );
        }
    }
}

/// The body of `name`, by matching braces from its declaration — never by a
/// text range between markers, which once swallowed a whole neighbouring
/// function that happened to sit between two being cut.
fn body_of(source: &str, name: &str) -> Option<String> {
    let at = source.find(&format!("fn {name}("))?;
    let open = source[at..].find('{')? + at;
    let mut depth = 0usize;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let body = &source[open + 1..open + offset];
                    return Some(body.split_whitespace().collect::<Vec<_>>().join(" "));
                }
            }
            _ => {}
        }
    }
    None
}

/// A hash may be carried rather than imported, but every copy must be the same
/// hash. They are identical today; drift would be invisible, since each shader
/// only ever compares its own rolls with its own.
#[test]
fn the_hash_every_shader_carries_is_the_same_hash() {
    let all = shaders();
    let cover = &all.iter().find(|(name, _)| name == COVER).expect("cover.wgsl").1;
    for helper in MAY_BE_CARRIED {
        let theirs = body_of(cover, helper).expect("cover.wgsl declares the hash");
        let mut carried = 0;
        for (name, source) in all.iter().filter(|(name, _)| name != COVER) {
            let Some(own) = body_of(source, helper) else { continue };
            carried += 1;
            assert_eq!(own, theirs, "{name} carries a different `{helper}` from cover.wgsl");
        }
        assert!(carried > 0, "nothing carries `{helper}` any more; drop it from MAY_BE_CARRIED");
    }
}

/// An import naming something `cover.wgsl` does not declare resolves to
/// nothing, with the same silence.
#[test]
fn nothing_imports_a_cover_rule_that_is_not_there() {
    let all = shaders();
    let cover = &all.iter().find(|(name, _)| name == COVER).expect("cover.wgsl").1;
    let rules = declared(cover);
    for (name, source) in all.iter().filter(|(name, _)| name != COVER) {
        for wanted in imported_from_cover(source) {
            assert!(
                rules.contains(&wanted),
                "{name} imports `{wanted}` from cover.wgsl, which does not declare it"
            );
        }
    }
}

/// The stages a way passes through are shared so that the ground, the grit
/// lying on it and what still grows in it turn together. A cover that hardcodes
/// one of the thresholds instead has its own schedule, which is the drift this
/// file exists to catch.
#[test]
fn the_wear_stages_are_named_in_one_place() {
    let all = shaders();
    let cover = &all.iter().find(|(name, _)| name == COVER).expect("cover.wgsl").1;
    for stage in ["WAY_BRUISED", "WAY_METALLED"] {
        assert!(cover.contains(&format!("const {stage}")), "cover.wgsl lost `{stage}`");
        for (name, source) in all.iter().filter(|(name, _)| name != COVER) {
            if source.contains(stage) {
                assert!(
                    imported_from_cover(source).contains(stage),
                    "{name} names `{stage}` without importing it"
                );
            }
        }
    }
}
