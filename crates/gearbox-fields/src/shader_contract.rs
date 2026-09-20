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
const MUST_AGREE: [&str; 18] = [
    "worn", "washed_into", "height_blend", "settled", "verge_damp", "way_beyond", "taken",
    "lattice", "clump_frame", "edge_stray", "inside_field", "rut_of", "way_read", "earth_mottle",
    "way_walk", "way_print", "wheel_print", "tyre_bars",
];

/// `pcg` and `rand` are not rules, only hashing, and a cover that does not wear
/// at all — the concrete yard — has no business importing the wear system for
/// them. They may be carried, but they may not *differ*.
const MAY_BE_CARRIED: [&str; 2] = ["pcg", "rand"];

/// The names a file declares: a `fn name(`, a `const name:`, or a `struct name`
/// — the cover shares a type as well as its rules now, since the line a wheel
/// drove is read from a shape the covers must all agree on.
fn declared(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line
                .strip_prefix("fn ")
                .or_else(|| line.strip_prefix("const "))
                .or_else(|| line.strip_prefix("struct "))?;
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

/// Naming the same rule twice in one import list is not a harmless repetition:
/// naga_oil rejects the whole file with "Ambiguous import path for item", and
/// since a cover whose shader will not build simply leaves the plain terrain
/// showing through, the field looks untextured rather than broken. The bare
/// ground imported `lattice` twice for long enough that every ploughed, dirt and
/// sand field rendered as flat terrain, with the reason only in the log.
#[test]
fn no_shader_imports_the_same_rule_twice() {
    for (name, source) in shaders() {
        for line in source.lines().filter(|line| line.contains("cover.wgsl\"::")) {
            let Some((_, rest)) = line.split_once('{') else { continue };
            let Some((names, _)) = rest.split_once('}') else { continue };
            let mut seen = BTreeSet::new();
            for wanted in names.split(',').map(str::trim).filter(|it| !it.is_empty()) {
                assert!(
                    seen.insert(wanted.to_owned()),
                    "{name} imports `{wanted}` twice from cover.wgsl; naga_oil calls that \
                     ambiguous and drops the whole shader"
                );
            }
        }
    }
}

/// A tyre's bars meet at its own centreline, and the print is a chevron because
/// of it. Taken from the *signed* offset instead, each rut prints parallel
/// diagonals — and since the offset is measured outward from the middle of the
/// way, the slope runs opposite ways in the two ruts, so one wheel leans one way
/// and the other the other. That is what the first attempt at this looked like,
/// and nothing about the picture says which of the two mistakes it is.
#[test]
fn the_tyre_bars_meet_at_the_centre_of_the_rut() {
    let all = shaders();
    let cover = &all.iter().find(|(name, _)| name == COVER).expect("cover.wgsl").1;
    let body = body_of(cover, "tyre_bars").expect("cover.wgsl presses a tyre's bars");
    let phase = body
        .split("let phase = ")
        .nth(1)
        .and_then(|it| it.split_once(';'))
        .map(|(it, _)| it.to_owned())
        .expect("the print has a phase");
    assert!(
        phase.contains("abs(across)"),
        "the bars are placed from `{phase}`; taken from the signed offset they are \
         diagonals, and mirrored between the two ruts"
    );
}

/// The print is relief and not paint: a cover that reads it must put it into the
/// normal it shades with. Tinting alone is the thing that was tried and thrown
/// away — at any strength that survives the stone lying on a rut it reads as
/// hatching, and at one that does not shout it cannot be seen at all.
#[test]
fn every_cover_that_prints_a_tyre_bar_tilts_its_normal_by_it() {
    for (name, source) in shaders() {
        if name == COVER || !calls(&source, "way_print") {
            continue;
        }
        assert!(
            source.contains("print.y") && source.contains("print.z"),
            "{name} reads the tyre print but never tilts a normal by it; drawn as a \
             tone alone it reads as hatching"
        );
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

/// One stage threshold is needed on the CPU as well: the hollow under a way is
/// cut there, and whether a way is crowned like a road or troughed like a track
/// has to be the same decision the colour makes. Two constants, one value.
#[test]
fn the_cpu_and_the_shader_agree_where_a_road_begins() {
    let all = shaders();
    let cover = &all.iter().find(|(name, _)| name == COVER).expect("cover.wgsl").1;
    let line = cover
        .lines()
        .find(|line| line.trim_start().starts_with("const WAY_METALLED"))
        .expect("cover.wgsl declares WAY_METALLED");
    let value: f32 = line
        .rsplit_once('=')
        .and_then(|(_, rest)| rest.trim().trim_end_matches(';').parse().ok())
        .expect("a number");
    assert_eq!(
        value,
        crate::layout::WAY_METALLED,
        "cover.wgsl says a road begins at {value}, the hollow says {}",
        crate::layout::WAY_METALLED
    );
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
