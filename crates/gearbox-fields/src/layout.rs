//! Validated scene field layout and rectangular region partitioning.

#[cfg(test)]
#[path = "layout/friction_tests.rs"]
mod friction_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_regions_cover_the_domain_without_overlap() {
        let layout: FieldLayout = serde_json::from_str(include_str!("layouts/mixed.json")).unwrap();
        let domain = FieldBounds {
            min: Vec2::splat(-400.0),
            max: Vec2::splat(400.0),
        };
        let regions = layout.regions(domain);
        let area: f32 = regions
            .iter()
            .map(|field| {
                let bounds = field.bounds();
                assert!(domain.contains(bounds.min) && domain.contains(bounds.max));
                let size = bounds.max - bounds.min;
                size.x * size.y
            })
            .sum();
        assert!((area - 640_000.0).abs() < 0.1);
        for (i, field) in regions.iter().enumerate() {
            for other in &regions[..i] {
                assert_ne!(field.name, other.name);
                assert!(!field.bounds().overlaps(other.bounds()));
            }
        }
    }

    #[test]
    fn default_layout_is_the_bundled_mixed_layout() {
        let domain = FieldBounds {
            min: Vec2::splat(-400.0),
            max: Vec2::splat(400.0),
        };
        let regions = FieldLayout::default().regions(domain);
        assert!(regions.iter().any(|field| field.profile == "grassland"));
        assert!(regions.iter().any(|field| field.profile == "harvested_wheat"));
    }

    // One point read back exactly as the shader's `way_point` reads it: two to
    // a column, the first eight from one matrix and the rest from the second.
    fn point_of(way: &Way, i: usize) -> Vec2 {
        let (first, more, _) = way.packed();
        let column = if i >= 8 { more.col((i - 8) / 2) } else { first.col(i / 2) };
        if i % 2 == 0 { column.xy() } else { column.zw() }
    }

    // The shader reads the points by position out of the matrix columns, so the
    // packing is a contract between two files and not an implementation detail.
    #[test]
    fn a_way_packs_two_points_to_a_column_of_two_matrices() {
        let points: Vec<Vec2> =
            (0..Way::MOST).map(|i| Vec2::new(i as f32, 10.0 + i as f32)).collect();
        let way = Way::bend(&points, 2.5, 0.5);
        let (_, _, shape) = way.packed();
        assert_eq!(shape.x, Way::MOST as f32);
        assert_eq!(shape.y, 2.5);
        for (i, point) in points.iter().enumerate() {
            assert_eq!(point_of(&way, i), *point, "point {i}");
        }
    }

    #[test]
    fn a_way_needs_two_points_and_keeps_at_most_sixteen() {
        assert_eq!(Way::bend(&[Vec2::ZERO], 1.0, 0.5).points(), 0);
        assert_eq!(Way::straight().points(), 0);
        let many: Vec<Vec2> = (0..20).map(|i| Vec2::splat(i as f32)).collect();
        assert_eq!(Way::bend(&many, 1.0, 0.5).points(), Way::MOST as u32);
    }

    // Points of a way, in order, read back the way a shader reads them.
    fn line_of(way: &Way) -> Vec<Vec2> {
        let (_, _, shape) = way.packed();
        (0..shape.x as usize).map(|i| point_of(way, i)).collect()
    }

    // The whole point of clipping rather than resampling: two fields either
    // side of a boundary must describe the road with the *same* points, or it
    // kinks where they meet. Nothing has to be told how far along the road it
    // lies — the ruts take their wander from the nearest point of the line,
    // which is a place in the world and so the same from either side.
    #[test]
    fn two_fields_clip_one_road_to_the_same_line() {
        let road: Vec<Vec2> = (0..12).map(|i| Vec2::new(i as f32 * 10.0, 0.0)).collect();
        let west = FieldBounds { min: Vec2::new(0.0, -20.0), max: Vec2::new(50.0, 20.0) };
        let east = FieldBounds { min: Vec2::new(50.0, -20.0), max: Vec2::new(110.0, 20.0) };
        for bounds in [west, east] {
            let way = Way::clipped(&road, 3.0, 0.5, bounds).unwrap();
            let kept = line_of(&way);
            // A contiguous run of the road itself — nothing moved or resampled.
            let at = road.iter().position(|p| *p == kept[0]).unwrap();
            assert_eq!(kept, road[at..at + kept.len()]);
        }
        let west_line = line_of(&Way::clipped(&road, 3.0, 0.5, west).unwrap());
        let east_line = line_of(&Way::clipped(&road, 3.0, 0.5, east).unwrap());
        let shared: Vec<Vec2> =
            west_line.iter().copied().filter(|p| east_line.contains(p)).collect();
        assert!(!shared.is_empty(), "the two fields must overlap on the road");
    }

    // The chunk cull tells a chunk it is unworn when the way does not reach it.
    // If that were ever stricter than the clip, wear would vanish from chunks
    // that should have it and the road would break into dashes.
    #[test]
    fn a_way_reaches_exactly_where_it_clips() {
        let road: Vec<Vec2> =
            (0..6).map(|i| Vec2::new(i as f32 * 20.0, (i % 2) as f32 * 8.0)).collect();
        let way = Way::bend(&road, 3.0, 0.5);
        for x in (-40..160).step_by(8) {
            for z in (-40..60).step_by(8) {
                let chunk = FieldBounds {
                    min: Vec2::new(x as f32, z as f32),
                    max: Vec2::new(x as f32 + 16.0, z as f32 + 16.0),
                };
                assert_eq!(
                    way.reaches(chunk),
                    Way::clipped(&road, 3.0, 0.5, chunk).is_some(),
                    "chunk at {x},{z}"
                );
            }
        }
    }

    // The hollow sinks the terrain grid, which is what the collider is built
    // from, so getting its shape wrong is felt and not just seen.
    #[test]
    fn a_hollow_is_deepest_on_the_way_and_nothing_off_it() {
        let layout: FieldLayout = serde_json::from_str(
            r#"{"default":"grassland","ways":[{"name":"lane","width":6.0,"wear":0.5,
                "points":[[0,0],[100,0]]}]}"#,
        )
        .unwrap();
        let hollows = Hollows::of(&layout);
        let on_it = hollows.depth_at(50.0, 0.0);
        // 0.08 + 0.5 * 0.35, the sink a half-worn way settles to.
        assert!((on_it - 0.255).abs() < 1e-5, "on the way: {on_it}");
        // A wheel must ride in and out, so the steepest side stays gentle.
        let slope = (0..60)
            .map(|step| {
                let at = step as f32 * 0.1;
                (hollows.depth_at(50.0, at) - hollows.depth_at(50.0, at + 0.1)).abs() / 0.1
            })
            .fold(0.0f32, f32::max);
        assert!(slope < 0.2, "sides of one in {:.0}", 1.0 / slope);
        // Level with the field again well outside the way and its sides.
        assert_eq!(hollows.depth_at(50.0, 3.0 + FADE_M + 0.1), 0.0);
        assert_eq!(hollows.depth_at(50.0, 400.0), 0.0);
        assert_eq!(hollows.depth_at(-400.0, 0.0), 0.0);
        // Sides that fall away, never a step.
        let mut last = on_it;
        for step in 1..40 {
            let here = hollows.depth_at(50.0, step as f32 * 0.15);
            assert!(here <= last + 1e-6, "the sides must not rise again");
            assert!(last - here < 0.05, "no step in the sides");
            last = here;
        }
        assert!(Hollows::of(&FieldLayout::default()).depth_at(0.0, 0.0) >= 0.0);
    }

    // The same track authored two ways — as a road over the layout, or as one
    // field's own way — must sink the ground identically, or a lane changes
    // depth depending on which file it happens to be written in.
    #[test]
    fn a_field_way_sinks_like_a_layout_road() {
        let road: FieldLayout = serde_json::from_str(
            r#"{"default":"grassland","ways":[{"name":"lane","width":6.0,"wear":0.7,
                "points":[[0,0],[100,0]]}]}"#,
        )
        .unwrap();
        let lane: FieldLayout = serde_json::from_str(
            r#"{"default":"grassland","fields":[{"name":"lane","profile":"track",
                "min":[0,-20],"max":[100,20],"wear":0.7,"way_width":6.0,
                "way":[[0,0],[100,0]]}]}"#,
        )
        .unwrap();
        let (a, b) = (Hollows::of(&road), Hollows::of(&lane));
        for step in 0..30 {
            let z = step as f32 * 0.3;
            assert_eq!(a.depth_at(50.0, z), b.depth_at(50.0, z), "at z={z}");
        }
        assert!(a.depth_at(50.0, 0.0) > 0.2);
    }

    // A track is a trough and traps its water; a made road is crowned and sheds
    // it. Which one a way is goes by the same number that decides whether stone
    // has come up through its fines, so the hollow and the colour cannot
    // disagree about what they are looking at.
    #[test]
    fn a_track_is_troughed_and_a_road_is_crowned() {
        let way = |wear: f32| {
            Hollows::of(
                &serde_json::from_str::<FieldLayout>(&format!(
                    r#"{{"default":"grassland","ways":[{{"name":"w","width":6.0,"wear":{wear},
                        "points":[[0,0],[100,0]]}}]}}"#
                ))
                .unwrap(),
            )
        };
        let (middle, shoulder) = (0.0, 2.4);
        let track = way(0.3);
        assert!(
            track.depth_at(50.0, middle) > track.depth_at(50.0, shoulder),
            "a soft track lies deepest down its middle"
        );
        let road = way(1.0);
        assert!(
            road.depth_at(50.0, shoulder) > road.depth_at(50.0, middle),
            "a made road lies deepest at its shoulders"
        );
        // Crowned or not, the whole way still sits below the field around it.
        assert!(road.depth_at(50.0, middle) > 0.0);
        assert_eq!(road.depth_at(50.0, 400.0), 0.0);
        // And it is still a road. The terrain grid is what the collider is built
        // from, so a camber that rose sharply would be a ridge a wheel climbs
        // rather than a fall it leans on. Measured across the running surface
        // only — past that the hollow's own sides take over, and those are as
        // steep as the way is deep whether it is cambered or not.
        let steepest = (0..15)
            .map(|step| {
                let at = step as f32 * 0.1;
                (road.depth_at(50.0, at) - road.depth_at(50.0, at + 0.1)).abs() / 0.1
            })
            .fold(0.0f32, f32::max);
        assert!(steepest < 0.10, "a road's camber falls one in {:.0}", 1.0 / steepest);
    }

    // The colour and the ground must agree about a crossroads: `worn()` adds
    // the lesser way to the greater there, and so must the hollow, or a
    // junction reads as churned but sits no lower than the roads either side.
    #[test]
    fn a_crossroads_is_dug_deeper_than_either_way() {
        let layout: FieldLayout = serde_json::from_str(
            r#"{"default":"grassland","ways":[
                {"name":"a","width":6.0,"wear":0.5,"points":[[-50,0],[50,0]]},
                {"name":"b","width":6.0,"wear":0.5,"points":[[0,-50],[0,50]]}]}"#,
        )
        .unwrap();
        let hollows = Hollows::of(&layout);
        let alone = hollows.depth_at(40.0, 0.0);
        let meeting = hollows.depth_at(0.0, 0.0);
        assert!((alone - 0.255).abs() < 1e-5, "one way alone: {alone}");
        assert!(meeting > alone * 1.5, "the crossing at {meeting} is no deeper");
        // Still gentle enough to drive in and out of, even dug twice.
        let slope = (0..80)
            .map(|step| {
                let at = step as f32 * 0.1;
                (hollows.depth_at(at, 0.0) - hollows.depth_at(at + 0.1, 0.0)).abs() / 0.1
            })
            .fold(0.0f32, f32::max);
        assert!(slope < 0.3, "sides of one in {:.0}", 1.0 / slope);
    }

    // A hollow has to belong to the way that made it. Sides of a fixed width
    // and a depth of a fixed depth turn a footpath into a broad shallow valley
    // several times its own width, which is not what a footpath wears.
    #[test]
    fn a_narrow_way_sinks_a_narrow_hollow() {
        let of = |width: f32| {
            let json = format!(
                r#"{{"default":"grassland","ways":[{{"name":"w","width":{width},"wear":0.4,
                    "points":[[0,0],[100,0]]}}]}}"#
            );
            Hollows::of(&serde_json::from_str::<FieldLayout>(&json).unwrap())
        };
        let reaches = |hollows: &Hollows| {
            (0..200).map(|s| s as f32 * 0.05).find(|z| hollows.depth_at(50.0, *z) <= 0.0).unwrap()
        };
        let (narrow, wide) = (of(1.2), of(6.0));
        // A foot-wide path must not dig wider than a lane does.
        assert!(reaches(&narrow) < 1.6, "a 1.2 m path sinks {} m out", reaches(&narrow));
        assert!(reaches(&narrow) < reaches(&wide) * 0.5);
        // And not as deep, either: a groove, not a sunken lane.
        assert!(narrow.depth_at(50.0, 0.0) < wide.depth_at(50.0, 0.0) * 0.6);
        // The wide one is untouched by any of that: 0.08 + 0.4 * 0.35, the sink
        // a soft track settles to, and no crown at that wear to lift its middle.
        assert!((wide.depth_at(50.0, 0.0) - 0.22).abs() < 1e-5);
        assert!(reaches(&wide) > 4.0);
    }

    #[test]
    fn a_field_with_no_way_of_its_own_sinks_nothing() {
        let plain: FieldLayout = serde_json::from_str(
            r#"{"default":"grassland","fields":[{"name":"plot","profile":"ploughed",
                "min":[0,0],"max":[100,100]}]}"#,
        )
        .unwrap();
        assert!(Hollows::of(&plain).is_empty());
    }

    // Two roads crossing one field share the sixteen points end to end, each
    // keeping its own width; the shader walks them as two separate lines.
    #[test]
    fn two_crossing_ways_share_the_points() {
        let north = Way::bend(&[Vec2::new(0.0, -50.0), Vec2::new(0.0, 50.0)], 3.0, 0.5);
        let east = Way::bend(&[Vec2::new(-50.0, 0.0), Vec2::new(0.0, 4.0), Vec2::new(50.0, 0.0)], 2.0, 0.5);
        let junction = north.crossing(east);
        let (_, _, shape) = junction.packed();
        assert_eq!((shape.x, shape.y), (2.0, 3.0));
        assert_eq!((shape.z, shape.w), (3.0, 2.0));
        assert_eq!(junction.points(), 5);
        // The first line's two points, then the second line's three after them.
        assert_eq!(point_of(&junction, 0), Vec2::new(0.0, -50.0));
        assert_eq!(point_of(&junction, 1), Vec2::new(0.0, 50.0));
        assert_eq!(point_of(&junction, 2), Vec2::new(-50.0, 0.0));
        assert_eq!(point_of(&junction, 4), Vec2::new(50.0, 0.0));
    }

    // A junction that spans both matrices: the second line must still be read
    // back correctly once its points run past the eighth.
    #[test]
    fn a_crossing_may_run_into_the_second_matrix() {
        let long: Vec<Vec2> = (0..7).map(|i| Vec2::new(i as f32 * 10.0, 0.0)).collect();
        let other: Vec<Vec2> = (0..6).map(|i| Vec2::new(30.0, i as f32 * 10.0 - 30.0)).collect();
        let junction = Way::bend(&long, 3.0, 0.5).crossing(Way::bend(&other, 2.0, 0.5));
        let (_, _, shape) = junction.packed();
        assert_eq!((shape.x, shape.z), (7.0, 6.0));
        for (i, point) in long.iter().chain(other.iter()).enumerate() {
            assert_eq!(point_of(&junction, i), *point, "point {i}");
        }
    }

    // Trimming either line to make room would leave two neighbouring fields
    // describing the same road differently, and it would kink between them.
    // The chunk cull asks `reaches`, and a chunk it says no to is told it has
    // no wear at all. Were that wrong for the *second* line, the crossing road
    // would keep its grass while the ground under it went bare.
    // An empty first line would read to a shader as no way at all, which means
    // the whole field worn down its long axis — the loudest failure there is.
    #[test]
    fn an_empty_way_never_leads_a_pair() {
        let real = Way::bend(&[Vec2::new(0.0, 0.0), Vec2::new(50.0, 0.0)], 3.0, 0.5);
        let nothing = Way::straight();
        assert_eq!(nothing.crossing(real), real);
        assert_eq!(real.crossing(nothing), real);
        assert_eq!(nothing.crossing(nothing), nothing);
        assert_eq!(real.crossing(nothing).packed().2.x, 2.0);
    }

    #[test]
    fn a_crossing_way_reaches_its_own_chunks() {
        let north = Way::bend(&[Vec2::new(0.0, -50.0), Vec2::new(0.0, 50.0)], 3.0, 0.5);
        let east = Way::bend(&[Vec2::new(-50.0, 30.0), Vec2::new(50.0, 30.0)], 2.0, 0.5);
        let junction = north.crossing(east);
        let chunk = |x: f32, z: f32| FieldBounds {
            min: Vec2::new(x, z),
            max: Vec2::new(x + 16.0, z + 16.0),
        };
        // Only the second line passes here, well away from the first.
        assert!(!north.reaches(chunk(32.0, 24.0)));
        assert!(junction.reaches(chunk(32.0, 24.0)));
        // Only the first line passes here.
        assert!(junction.reaches(chunk(-8.0, -40.0)));
        // Neither does here.
        assert!(!junction.reaches(chunk(32.0, -40.0)));
    }

    // Two lines is all a way holds, so a third road over the same field is
    // dropped from the *wear* — but `Hollows` knows nothing of that limit and
    // sinks the ground along all three. A triple crossing therefore leaves a
    // shallow grassy trough where the third road should have been painted.
    /// Only the default layout is compiled in; the rest are read from a path at
    /// run time, so a typo in one of them used to show up as a sim that would
    /// not start rather than as a failing test. `deny_unknown_fields` makes this
    /// worth more than a parse: a key nobody reads is caught here.
    #[test]
    fn every_bundled_layout_parses() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/layouts");
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).expect("the bundled layouts") {
            let path = entry.expect("a layout").path();
            if path.extension().is_none_or(|it| it != "json") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("readable");
            let layout: FieldLayout = serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(
                !layout.fields.is_empty() || !layout.default.is_empty(),
                "{}: a layout with neither fields nor a default covers nothing",
                path.display()
            );
            seen += 1;
        }
        assert!(seen > 5, "found only {seen} layouts; the walk is wrong");
    }

    /// The print belongs to whatever drives the way. A lorry road and a farm
    /// track crossing the same field must reach the shader as two different
    /// tyres, or the ground decides what everything that drives on it leaves
    /// behind — which is how the chevron came to be printed by everything.
    #[test]
    fn each_line_carries_its_own_tyre() {
        let lane = Way::bend(&[Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)], 3.0, 0.6)
            .printed_by(TyreTread::AG);
        let metalled = Way::bend(&[Vec2::new(0.0, -50.0), Vec2::new(0.0, 50.0)], 3.0, 0.9)
            .printed_by(TyreTread::ROAD);
        let (first, second) = lane.crossing(metalled).bars();
        assert_eq!(first, TyreTread::AG.packed(), "the farm track lost its lug");
        assert_eq!(second, TyreTread::ROAD.packed(), "the road took the tractor's tread");
        // Leaning at all is what makes a chevron; the sign of the lean is only
        // which way round the V points.
        assert!(first.y.abs() > 0.5, "an ag lug leans, and that is what makes the chevron");
        assert_eq!(second.y, 0.0, "a lorry prints square across, never a chevron");
        assert!(second.x < first.x, "a road tyre's blocks are closer than an ag lug's bars");
    }

    /// A tyre with no pattern left prints none: the depth is what the shader
    /// skips on, so a bald tyre costs nothing rather than printing a flat bar.
    #[test]
    fn a_bald_tyre_prints_nothing() {
        assert_eq!(TyreTread::SMOOTH.packed().w, 0.0);
        assert!(TyreTread::AG.packed().w > 0.0);
    }

    #[test]
    fn a_tyre_nobody_has_heard_of_is_an_ag_lug() {
        let named = |name: &str| TreadSpec::Named(name.into()).tread();
        assert_eq!(named("ag"), TyreTread::AG);
        assert_eq!(named("lorry"), TyreTread::ROAD);
        assert_eq!(named("bald"), TyreTread::SMOOTH);
        assert_eq!(named("penny-farthing"), TyreTread::AG);
    }

    #[test]
    fn a_third_way_over_one_field_is_sunk_but_not_worn() {
        let north = Way::bend(&[Vec2::new(0.0, -50.0), Vec2::new(0.0, 50.0)], 3.0, 0.5);
        let east = Way::bend(&[Vec2::new(-50.0, 0.0), Vec2::new(50.0, 0.0)], 3.0, 0.5);
        let slant = Way::bend(&[Vec2::new(-40.0, -40.0), Vec2::new(40.0, 40.0)], 3.0, 0.5);
        let three = north.crossing(east).crossing(slant);
        // The third is refused, and refusing it leaves the first two whole.
        assert_eq!(three, north.crossing(east));
        assert_eq!(three.packed().2.x, 2.0);
        assert_eq!(three.packed().2.z, 2.0);

        let layout: FieldLayout = serde_json::from_str(
            r#"{"default":"grassland","ways":[
                {"name":"a","width":6,"wear":0.5,"points":[[0,-50],[0,50]]},
                {"name":"b","width":6,"wear":0.5,"points":[[-50,0],[50,0]]},
                {"name":"c","width":6,"wear":0.5,"points":[[-40,-40],[40,40]]}]}"#,
        )
        .unwrap();
        // All three are sunk, so the ground dips where the wear will not paint.
        assert!(Hollows::of(&layout).depth_at(30.0, 30.0) > 0.2, "the third way is not sunk");
    }

    #[test]
    fn a_crossing_that_does_not_fit_is_left_out_whole() {
        let long: Vec<Vec2> = (0..10).map(|i| Vec2::new(i as f32 * 10.0, 0.0)).collect();
        let other: Vec<Vec2> = (0..9).map(|i| Vec2::new(20.0, i as f32 * 10.0 - 40.0)).collect();
        let first = Way::bend(&long, 3.0, 0.5);
        let joined = first.crossing(Way::bend(&other, 2.0, 0.5));
        assert_eq!(joined, first);
        assert_eq!(joined.packed().2.z, 0.0);
    }

    // A layout is the one place a mistake can be named. Past it these numbers
    // reach the terrain grid and the shaders, where a bad one is a silently
    // wrong ground or a collider full of holes.
    #[test]
    fn a_layout_names_what_is_wrong_with_its_ways() {
        // A profile that grows nothing and builds no ground: `validate` only
        // ever looks up its name, its border and its colour.
        struct NoGround;
        impl crate::profile::GroundSurface for NoGround {
            fn apply(&self, _: &mut Commands, _: Entity) {}
        }
        fn no_ground(
            _: &mut World,
            _: Handle<Image>,
            _: crate::profile::WheelMapParams,
            _: crate::profile::SurfaceGeometry,
            _: crate::profile::Placed,
        ) -> std::sync::Arc<dyn crate::profile::GroundSurface> {
            std::sync::Arc::new(NoGround)
        }
        let mut profiles = FieldProfiles::default();
        profiles.register(crate::profile::FieldProfile {
            name: "grassland",
            wheel_response: crate::profile::WheelResponse {
                recovery_seconds: 1.0,
                bend: 0.0,
                darkening: 0.0,
                footprint_length: 0.1,
                tread: false,
            },
            layers: Vec::new(),
            ground: no_ground,
            tread: Vec4::ZERO,
            surface_tint: Vec4::ONE,
            soft_border: 1.0,
        });
        let refuses = |json: &str| {
            serde_json::from_str::<FieldLayout>(json)
                .expect("valid json")
                .validate(&profiles)
                .expect_err("should have been refused")
        };
        // The layout with nothing wrong with it must pass, or every assertion
        // below could be objecting to something else entirely.
        let sound = r#"{"default":"grassland","ways":[
            {"name":"a","width":6,"wear":0.5,"points":[[0,0],[9,0]]}]}"#;
        serde_json::from_str::<FieldLayout>(sound).unwrap().validate(&profiles).unwrap();

        let with = |ways: &str| format!(r#"{{"default":"grassland","ways":[{ways}]}}"#);
        assert!(
            refuses(&with(r#"{"name":"a","width":6,"wear":0.5,"points":[[0,0]]}"#))
                .contains("two points")
        );
        assert!(
            refuses(&with(r#"{"name":"a","width":0,"wear":0.5,"points":[[0,0],[9,0]]}"#))
                .contains("0 m wide")
        );
        // JSON cannot write NaN, but it can write a number too big to hold,
        // which arrives as an infinity and would sink the terrain to nothing.
        assert!(
            refuses(&with(r#"{"name":"a","width":6,"wear":0.5,"points":[[1e40,0],[9,0]]}"#))
                .contains("not a number")
        );
        assert!(
            refuses(&with(r#"{"name":"a","width":6,"wear":4,"points":[[0,0],[9,0]]}"#))
                .contains("not nought to one")
        );
        assert!(
            refuses(&with(
                r#"{"name":"a","width":6,"wear":0.5,"points":[[0,0],[9,0]]},
                   {"name":"a","width":6,"wear":0.5,"points":[[0,9],[9,9]]}"#
            ))
            .contains("duplicate way name")
        );
    }

    #[test]
    fn a_road_that_misses_a_field_clips_to_nothing() {
        let road = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
        let away = FieldBounds { min: Vec2::new(0.0, 60.0), max: Vec2::new(100.0, 90.0) };
        assert!(Way::clipped(&road, 3.0, 0.5, away).is_none());
    }

    #[test]
    fn a_layout_way_defaults_to_the_field_width() {
        let spec: FieldSpec = serde_json::from_str(
            r#"{"name":"lane","profile":"track","min":[0,0],"max":[10,100],
                "way":[[5,0],[5,100]]}"#,
        )
        .unwrap();
        assert_eq!(spec.way().packed().2.y, 5.0);
    }
}

use super::profile::FieldProfiles;
use bevy::prelude::*;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldBounds {
    pub min: Vec2,
    pub max: Vec2,
}

impl Default for FieldBounds {
    fn default() -> Self {
        Self { min: Vec2::ZERO, max: Vec2::ZERO }
    }
}

impl FieldBounds {
    pub fn contains(&self, point: Vec2) -> bool {
        point.cmpge(self.min).all() && point.cmple(self.max).all()
    }

    pub fn overlaps(&self, other: Self) -> bool {
        self.min.cmplt(other.max).all() && other.min.cmplt(self.max).all()
    }

    pub fn nearest_distance(&self, point: Vec2) -> f32 {
        point.distance(point.clamp(self.min, self.max))
    }

    /// The same rectangle with `by` metres of margin on every side.
    pub fn grown(&self, by: f32) -> Self {
        Self { min: self.min - Vec2::splat(by), max: self.max + Vec2::splat(by) }
    }
}

/// How far a field's edge may stray from the line it was authored on, and so
/// how much ground each field must be given *past* that line. The shaders hold
/// the same number as `EDGE_STRAY_M` in `bare/shaders/cover.wgsl` and cut the
/// ground on the strayed edge; this is what makes sure there is ground there to
/// cut. Short of it, a field ends before its own edge does and the terrain
/// shows through; the margin is deliberately the larger of the two.
pub const EDGE_MARGIN_M: f32 = 0.75;

/// The line the wheels follow through a field, in world XZ. A field is always a
/// rectangle, so without this a way can only run straight down one; with it the
/// field is merely the corridor a track winds along inside.
///
/// Sixteen points hold up to two lines, laid end to end, each with its own
/// width and its own wear: where two roads cross a field, the ground is worn by
/// whichever of them has taken more of it, and a faint track may join a made
/// road without either becoming the other. The second is empty for the ordinary
/// case of one road.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Way {
    points: [Vec2; Way::MOST],
    lines: [Line; 2],
}

/// The mark a tyre leaves, which belongs to the tyre and not to the ground it
/// is left on. A tractor prints the 45° chevron of an agricultural lug; a lorry
/// prints fine bars straight across; a trailer tyre run bald prints nothing at
/// all. The ground's part is to be soft enough to take it — what shape it takes
/// is none of the ground's business.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TyreTread {
    /// Metres from one bar to the next along the direction of travel. A rear
    /// tractor tyre carries something like two dozen lugs round five metres of
    /// circumference; a lorry's blocks are far closer together.
    pub pitch: f32,
    /// How far a bar runs along the way for every metre it runs across it.
    /// Nought is a bar square across the tread, one is the 45° of an ag lug —
    /// and because the bars are placed from the distance out of the tyre's own
    /// middle, anything but nought meets there as a chevron.
    pub lean: f32,
    /// What share of the pitch the bar itself takes, the rest being the void
    /// between. An ag tyre is mostly void, so the soil it picks up can drop out
    /// of it; a road tyre is mostly rubber.
    pub duty: f32,
    /// How deep it presses into soft ground, in metres. Nought is a tyre that
    /// leaves no pattern — worn smooth, or never having had one.
    pub depth: f32,
}

impl TyreTread {
    /// An agricultural lug: the 45° chevron, widely spaced, pressing deep.
    pub const AG: Self = Self { pitch: 0.21, lean: -1.0, duty: 0.34, depth: 0.022 };

    /// A lorry or a car: close bars square across the tread, barely biting.
    pub const ROAD: Self = Self { pitch: 0.085, lean: 0.0, duty: 0.55, depth: 0.006 };

    /// A tyre that leaves no pattern, only the rut.
    pub const SMOOTH: Self = Self { pitch: 0.2, lean: 0.0, duty: 0.0, depth: 0.0 };

    /// The four numbers a shader prints from.
    pub fn packed(&self) -> Vec4 {
        Vec4::new(
            self.pitch.clamp(0.02, 2.0),
            self.lean.clamp(-4.0, 4.0),
            self.duty.clamp(0.0, 1.0),
            self.depth.clamp(0.0, 0.2),
        )
    }
}

impl Default for TyreTread {
    fn default() -> Self {
        Self::AG
    }
}

/// What a layout may write in place of the four numbers: a name for a tyre
/// anyone would recognise, or the numbers themselves for one nobody would.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum TreadSpec {
    Named(String),
    Given(TyreTread),
}

impl TreadSpec {
    /// The tread itself. An unknown name is said out loud and treated as the
    /// agricultural one, since a silent fallback here prints the wrong tyre for
    /// the rest of the layout's life.
    pub fn tread(&self) -> TyreTread {
        match self {
            Self::Given(tread) => *tread,
            Self::Named(name) => match name.as_str() {
                "ag" | "tractor" => TyreTread::AG,
                "road" | "lorry" | "car" | "truck" => TyreTread::ROAD,
                "smooth" | "bald" | "none" => TyreTread::SMOOTH,
                other => {
                    warn!("no tyre called `{other}`; the way is printed by an ag lug");
                    TyreTread::AG
                }
            },
        }
    }
}

/// One line's share of the points: how many of them, how wide it is worn, how
/// hard, and what tyre prints it. The wear belongs to the line and not to the
/// field, or a farm track crossing a metalled lane would take that lane's wear
/// and the pair of them read as one road — the fields a way crosses have
/// nothing to say about how used that way is. Nor about what uses it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Line {
    count: u32,
    half_width: f32,
    wear: f32,
    tread: TyreTread,
}

impl Way {
    /// Sixteen points, two to a column of two matrices. Eight was one matrix
    /// and too few: a road crossing a background region keeps most of its
    /// points, so two five-point roads already overflowed a junction.
    pub const MOST: usize = 16;

    /// Nothing, for a field whose wear runs down its own long axis.
    pub fn straight() -> Self {
        Self::default()
    }

    /// How many points bend it; fewer than two is no way at all.
    pub fn points(&self) -> u32 {
        self.lines[0].count + self.lines[1].count
    }

    /// The two of them joined, if their points fit in the eight there are. The
    /// crossing road is dropped when they do not, because trimming either line
    /// would leave two neighbouring fields describing it differently and the
    /// road would kink on the boundary between them.
    pub fn crossing(self, other: Self) -> Self {
        let (mine, theirs) = (self.lines[0].count as usize, other.lines[0].count as usize);
        // A way whose *first* line is empty reads to a shader as no way at all,
        // and it falls back to wearing the whole field down its long axis. So
        // an empty one never becomes the first of a pair.
        if theirs < 2 {
            return self;
        }
        if mine < 2 {
            return other;
        }
        // Two reasons to refuse, and they want telling apart: one is answered by
        // cutting the field so each crossing has its own, the other by spending
        // fewer points on the roads. Reported as one, every third way over a
        // field read as a point-budget problem and sent the reader to count
        // points that were never the trouble.
        if self.lines[1].count > 0 || other.lines[1].count > 0 {
            warn!("a third way crosses here; a field wears along two, so this one is left out");
            return self;
        }
        if mine + theirs > Self::MOST {
            warn!(
                "two ways cross here needing {} points of {}; the second is left out",
                mine + theirs,
                Self::MOST
            );
            return self;
        }
        let mut joined = self;
        joined.points[mine..mine + theirs].copy_from_slice(&other.points[..theirs]);
        joined.lines[1] = other.lines[0];
        joined
    }

    /// Fewer than two points is no way at all; beyond `MOST` the tail is cut,
    /// which is said out loud rather than authored points going quietly missing.
    pub fn bend(points: &[Vec2], half_width: f32, wear: f32) -> Self {
        let mut way = Self::default();
        way.lines[0].half_width = half_width;
        way.lines[0].wear = wear.clamp(0.0, 1.0);
        if points.len() < 2 {
            return way;
        }
        if points.len() > Self::MOST {
            warn!(
                "a way of {} points keeps only its first {}; the rest of it is cut",
                points.len(),
                Self::MOST
            );
        }
        let taken = points.len().min(Self::MOST);
        way.points[..taken].copy_from_slice(&points[..taken]);
        way.lines[0].count = taken as u32;
        way
    }

    /// The stretch of a longer road that `bounds` can see, or `None` where the
    /// road does not come near this field at all. Both sides of a boundary clip
    /// the *same* points, so the two halves of a road meet exactly; only the
    /// ends are cut, never moved, and nothing is resampled.
    pub fn clipped(points: &[Vec2], half_width: f32, wear: f32, bounds: FieldBounds) -> Option<Self> {
        if points.len() < 2 {
            return None;
        }
        // Far enough out that the verge and its noise are still decided by the
        // real line and not by where the clip happened to fall.
        let reach = Vec2::splat(half_width + 2.0);
        let (min, max) = (bounds.min - reach, bounds.max + reach);
        let seen: Vec<usize> = (0..points.len() - 1)
            .filter(|&i| segment_meets(points[i], points[i + 1], min, max))
            .collect();
        let (&first, &last) = (seen.first()?, seen.last()?);
        // One leg beyond each end, so the way still has a direction at the
        // field's edge instead of stopping dead on it.
        let start = first.saturating_sub(1);
        let end = (last + 2).min(points.len() - 1);
        let kept = &points[start..=end];
        if kept.len() > Self::MOST {
            warn!(
                "way of {} points needs {} of them over [{:?}..{:?}]; only {} fit, so the \
                 road is cut short there — split the field or use fewer points",
                points.len(), kept.len(), bounds.min, bounds.max, Self::MOST
            );
        }
        Some(Self::bend(kept, half_width, wear))
    }

    /// Whether either line comes near enough to `bounds` to wear any of it. The
    /// shader's search costs a loop per blade and per pixel, so a chunk the
    /// road never touches is told it has no wear at all and pays one compare.
    pub fn reaches(&self, bounds: FieldBounds) -> bool {
        let mut at = 0;
        self.lines.iter().any(|line| {
            let from = at;
            at += line.count as usize;
            let reach = Vec2::splat(line.half_width + 2.0);
            let (min, max) = (bounds.min - reach, bounds.max + reach);
            (from..at.saturating_sub(1))
                .any(|i| segment_meets(self.points[i], self.points[i + 1], min, max))
        })
    }

    /// For a shader: the points two to a column of two matrices, the first
    /// eight then the rest, and how many of them each line takes with how wide
    /// it is worn.
    pub fn packed(&self) -> (Mat4, Mat4, Vec4) {
        let pair = |i: usize| {
            let (a, b) = (self.points[i * 2], self.points[i * 2 + 1]);
            Vec4::new(a.x, a.y, b.x, b.y)
        };
        let matrix = |from: usize| {
            Mat4::from_cols(pair(from), pair(from + 1), pair(from + 2), pair(from + 3))
        };
        (
            matrix(0),
            matrix(4),
            Vec4::new(
                self.lines[0].count as f32,
                self.lines[0].half_width,
                self.lines[1].count as f32,
                self.lines[1].half_width,
            ),
        )
    }

    /// What the shaders read wear from: the gauge a tractor's wheels sit at and
    /// how wide one rut is, then how hard each of the two lines is worn. A
    /// field with no line of its own falls back to `plain`, which wears it
    /// evenly down its long axis; with no line and no `plain` it is not worn.
    pub fn tread(&self, plain: Option<f32>) -> Vec4 {
        let second = match self.lines[1].count >= 2 {
            true => self.lines[1].wear,
            false => 0.0,
        };
        let first = match self.lines[0].count >= 2 {
            true => self.lines[0].wear,
            false => plain.unwrap_or(0.0),
        };
        Vec4::new(HALF_GAUGE_M, HALF_RUT_M, first, second)
    }

    /// The tyre that prints each of the two lines, for a shader.
    pub fn bars(&self) -> (Vec4, Vec4) {
        (self.lines[0].tread.packed(), self.lines[1].tread.packed())
    }

    /// The tyre that prints this way. Set after bending or clipping, so the
    /// long signatures those already carry do not grow a fourth thing that is
    /// nothing to do with where the line runs.
    pub fn printed_by(mut self, tread: TyreTread) -> Self {
        self.lines[0].tread = tread;
        self
    }
}

/// Half the gauge a tractor's wheels sit at, and half the width of one rut.
/// Shared by every way: the machines are the same whatever they are driving on,
/// so only how hard a way is worn tells one from another.
const HALF_GAUGE_M: f32 = 0.9;
const HALF_RUT_M: f32 = 0.46;

/// What every way must be, whether a field names it or the layout lays it over
/// them all: real points, a width to wear and a wear between nothing and bare.
/// A width or a wear left out is not checked: the field supplies its own, from
/// bounds this has already made sure of.
fn check_way(
    name: &str,
    points: &[[f32; 2]],
    width: Option<f32>,
    wear: Option<f32>,
) -> Result<(), String> {
    if points.iter().flatten().any(|at| !at.is_finite()) {
        return Err(format!("way of {name} has a point that is not a number"));
    }
    if let Some(width) = width
        && (!width.is_finite() || width <= 0.0)
    {
        return Err(format!("way of {name} is {width} m wide"));
    }
    match wear {
        Some(wear) if !wear.is_finite() || !(0.0..=1.0).contains(&wear) => {
            Err(format!("wear of {name} is {wear}, not nought to one"))
        }
        _ => Ok(()),
    }
}

/// Whether a segment comes inside an axis-aligned box, by clipping it against
/// each pair of slabs in turn.
fn segment_meets(a: Vec2, b: Vec2, min: Vec2, max: Vec2) -> bool {
    let run = b - a;
    let (mut enter, mut leave) = (0.0f32, 1.0f32);
    for axis in 0..2 {
        if run[axis].abs() < 1e-6 {
            if a[axis] < min[axis] || a[axis] > max[axis] {
                return false;
            }
            continue;
        }
        let near = (min[axis] - a[axis]) / run[axis];
        let far = (max[axis] - a[axis]) / run[axis];
        enter = enter.max(near.min(far));
        leave = leave.min(near.max(far));
        if enter > leave {
            return false;
        }
    }
    true
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    pub name: String,
    pub profile: String,
    pub min: [f32; 2],
    pub max: [f32; 2],
    /// How hard this field is worn, nought to one: a green lane barely marked,
    /// two bare ruts with grass between them, or bare across its width. Left
    /// out, the profile decides.
    #[serde(default)]
    pub wear: Option<f32>,
    /// World XZ points of the line the wheels follow, at most eight. Left out,
    /// the wear runs straight down the field's long axis.
    #[serde(default)]
    pub way: Vec<[f32; 2]>,
    /// How wide the worn corridor is, in metres. Left out, it is as wide as the
    /// field is across.
    #[serde(default)]
    pub way_width: Option<f32>,
    /// What drives it: "ag", "road" or "smooth", or the four numbers of a tyre
    /// nobody has a name for. Left out, a tractor.
    #[serde(default)]
    pub tyre: Option<TreadSpec>,
    /// Dimensionless ground friction; absent values inherit the layout default.
    #[serde(default)]
    pub friction: Option<f64>,
}

impl FieldSpec {
    pub fn bounds(&self) -> FieldBounds {
        FieldBounds {
            min: Vec2::from_array(self.min),
            max: Vec2::from_array(self.max),
        }
    }

    pub fn way(&self) -> Way {
        let (line, width, wear) = self.laid_way();
        Way::bend(&line, width * 0.5, wear).printed_by(self.tyre())
    }

    /// The tyre that prints this field's own way.
    pub fn tyre(&self) -> TyreTread {
        self.tyre.as_ref().map_or_else(TyreTread::default, TreadSpec::tread)
    }

    /// The line this field lays down for itself, how wide it is worn and how
    /// hard — the same three things a layout's road carries, so the two sink
    /// the ground alike. A field that names a way but no wear is taken as
    /// half worn, since only its profile knows better and this does not.
    fn laid_way(&self) -> (Vec<Vec2>, f32, f32) {
        let span = self.bounds().max - self.bounds().min;
        let across = span.x.min(span.y);
        (
            self.way.iter().copied().map(Vec2::from_array).collect(),
            self.way_width.unwrap_or(across),
            self.wear.unwrap_or(0.5),
        )
    }
}

/// A road laid across the whole layout rather than down one field. Every field
/// it passes over is worn along it, whatever that field grows, so a track can
/// run from one end of the country to the other without the fields being cut
/// up to describe it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaySpec {
    pub name: String,
    /// World XZ points of the line, as many as the road needs.
    pub points: Vec<[f32; 2]>,
    /// How wide the worn corridor is, in metres.
    pub width: f32,
    /// How hard it is worn, nought to one, for the fields it crosses that do
    /// not say for themselves.
    pub wear: f32,
    /// What drives it: "ag", "road" or "smooth", or the four numbers of a tyre
    /// nobody has a name for. Left out, a tractor.
    #[serde(default)]
    pub tyre: Option<TreadSpec>,
}

impl WaySpec {
    fn line(&self) -> Vec<Vec2> {
        self.points.iter().copied().map(Vec2::from_array).collect()
    }

    /// The stretch of this road a field can see, if it crosses that field.
    pub fn across(&self, bounds: FieldBounds) -> Option<Way> {
        let tyre = self.tyre.as_ref().map_or_else(TyreTread::default, TreadSpec::tread);
        Way::clipped(&self.line(), self.width * 0.5, self.wear, bounds)
            .map(|way| way.printed_by(tyre))
    }
}

/// How deep the ways of a layout sit below the ground around them, as a plain
/// function of world XZ. A track used for years is a hollow, not a stripe of
/// colour on a flat field: this is what lets one break the skyline at a
/// grazing angle, and what a wheel feels when it drops into one — the terrain
/// grid carries the collider, so sinking it here sinks it for the physics too.
///
/// Only an authored line sinks anything. A field that merely carries a `wear`
/// is worn across its whole width, and that is a shading of the ground, not a
/// trench dug down the middle of it: guessing which wide fields were meant to
/// be lanes would sink a ploughed plot along its long axis.
#[derive(Clone, Debug, Default)]
pub struct Hollows(Vec<Hollow>);

#[derive(Clone, Debug)]
struct Hollow {
    line: Vec<Vec2>,
    half: f32,
    sink: f32,
    camber: f32,
    fade: f32,
    min: Vec2,
    max: Vec2,
}

impl Hollow {
    fn new((line, width, wear): (Vec<Vec2>, f32, f32)) -> Option<Self> {
        if line.len() < 2 || width <= 0.0 {
            return None;
        }
        let half = width * 0.5;
        // A hollow belongs to the way that made it: a footpath wears a groove,
        // not a broad shallow valley several times its own width. Both the
        // sides and the depth are held back for a way too narrow to carry them,
        // and neither is touched for anything a tractor fits down.
        let fade = FADE_M.min(half * 0.8);
        let slight = (half / 1.5).min(1.0);
        let reach = Vec2::splat(half + fade);
        let fold = |pick: fn(Vec2, Vec2) -> Vec2| line.iter().copied().reduce(pick).unwrap();
        Some(Self {
            half,
            fade,
            // Barely marked where a lane is hardly worn, a proper sunken way
            // where it is a road. The sides stay about one in five even at its
            // deepest, so a wheel rides in and out of it rather than catching
            // on the lip.
            sink: (0.08 + wear.clamp(0.0, 1.0) * 0.35) * slight,
            // A worn track is a trough down its middle and traps the water that
            // falls on it; a *made* road is cambered so it sheds it to the
            // shoulder, and losing that camber is how a gravel road fails. The
            // same number decides both, because it is the same number that
            // decides whether stone has come up through the fines.
            //
            // Four per cent of cross-fall, which is what an unpaved road is
            // built to, taken as a share of the way's own width because a wide
            // road and a narrow one are laid to the same slope. Scaled by the
            // *sink* instead it came out at one in four — a ridge a wheel
            // climbs rather than a camber it leans on.
            camber: 0.04 * half * smoothstep(WAY_METALLED, 0.95, wear.clamp(0.0, 1.0)),
            min: fold(Vec2::min) - reach,
            max: fold(Vec2::max) + reach,
            line,
        })
    }
}

impl Hollows {
    /// A metre of cell means the ruts themselves can never be geometry; the
    /// trough the whole way sits in is several metres across and can.
    pub fn of(layout: &FieldLayout) -> Self {
        // A road laid over the layout and a way a single field names for itself
        // are the same thing authored at two scales; both sink what they cross.
        let roads = layout.ways.iter().map(|way| (way.line(), way.width, way.wear));
        let lanes = layout.fields.iter().map(FieldSpec::laid_way);
        Self(roads.chain(lanes).filter_map(Hollow::new).collect())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How far the ground drops here, in metres; nought away from every way.
    pub fn depth_at(&self, x: f32, z: f32) -> f32 {
        let place = Vec2::new(x, z);
        let mut deepest = 0.0f32;
        for hollow in &self.0 {
            // Nearly every point of a square kilometre is nowhere near a road,
            // and the walk down its legs is far dearer than four compares.
            if place.cmplt(hollow.min).any() || place.cmpgt(hollow.max).any() {
                continue;
            }
            let mut nearest = f32::MAX;
            for leg in hollow.line.windows(2) {
                let run = leg[1] - leg[0];
                let length = run.length().max(1e-4);
                let at = (place - leg[0]).dot(run / length).clamp(0.0, length);
                nearest = nearest.min(place.distance(leg[0] + run / length * at));
            }
            let sides = 1.0 - smoothstep(hollow.half * 0.55, hollow.half + hollow.fade, nearest);
            // Across the way itself: a worn track lies deepest down its middle,
            // a made road stands proud there and lies deepest at its shoulders,
            // which is the 4 to 6 per cent of cross-slope that sheds the water
            // off it rather than holding it.
            let out_by = (nearest / hollow.half.max(0.05)).min(1.0);
            let crown = hollow.camber * (1.0 - smoothstep(0.0, 0.9, out_by));
            // Where two ways meet, the lesser adds to the greater rather than
            // hiding under it — the same rule `worn()` uses for the colour, so
            // a crossroads is dug out as well as worn bare.
            let here = (hollow.sink - crown) * sides;
            deepest = deepest.max(here) + deepest.min(here) * 0.6;
        }
        deepest
    }
}

/// Where stone begins to come up through the fines: the point a way stops
/// being a soft track and becomes a made road. Held here as well as in
/// `bare/shaders/cover.wgsl`, because the hollow is cut on the CPU and the
/// colour on the GPU and the two must agree on which a way is;
/// `shader_contract` refuses to let the two drift apart.
pub const WAY_METALLED: f32 = 0.55;

/// How far out a hollow's sides run past the worn part of the way.
const FADE_M: f32 = 1.6;

fn smoothstep(from: f32, to: f32, at: f32) -> f32 {
    let t = ((at - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Resource, Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldLayout {
    pub default: String,
    /// Dimensionless background friction; absent values use the ground collider.
    #[serde(default)]
    pub default_friction: Option<f64>,
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
    /// Roads laid over the fields; the first one a field meets wears it.
    #[serde(default)]
    pub ways: Vec<WaySpec>,
}

impl Default for FieldLayout {
    fn default() -> Self {
        // Grass everywhere, with a harvested-wheat stubble region cut into
        // it — the bundled `mixed.json` is the single source of truth for
        // this so the default layout can't drift from its own test fixture.
        serde_json::from_str(include_str!("layouts/mixed.json"))
            .expect("bundled default field layout must be valid")
    }
}

impl FieldLayout {
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("GEARBOX_FIELD_LAYOUT") {
            Ok(path) => {
                let contents = std::fs::read_to_string(&path)
                    .map_err(|error| format!("field layout {path}: {error}"))?;
                serde_json::from_str(&contents)
                    .map_err(|error| format!("field layout {path}: {error}"))
            }
            Err(std::env::VarError::NotPresent) => Ok(Self::default()),
            Err(error) => Err(format!("GEARBOX_FIELD_LAYOUT: {error}")),
        }
    }

    pub fn validate(&self, profiles: &FieldProfiles, terrain: FieldBounds) -> Result<(), String> {
        self.validate_bounds(terrain)?;
        if !profiles.0.contains_key(&self.default) {
            return Err(format!("unknown default field profile: {}", self.default));
        }
        for field in &self.fields {
            if !profiles.0.contains_key(&field.profile) {
                return Err(format!("unknown field profile: {}", field.profile));
            }
        }
        Ok(())
    }

    fn validate_bounds(&self, terrain: FieldBounds) -> Result<(), String> {
        if !terrain.min.is_finite()
            || !terrain.max.is_finite()
            || !terrain.min.cmplt(terrain.max).all()
        {
            return Err("invalid field terrain bounds".into());
        }
        for friction in self
            .default_friction
            .iter()
            .chain(self.fields.iter().filter_map(|field| field.friction.as_ref()))
        {
            if !friction.is_finite() || *friction < 0.0 {
                return Err("field friction must be finite and non-negative".into());
            }
        }
        for (i, field) in self.fields.iter().enumerate() {
            let bounds = field.bounds();
            if field.name.trim().is_empty()
                || field.name.starts_with("__background/")
                || self.fields[..i].iter().any(|f| f.name == field.name)
            {
                return Err(format!("empty or duplicate field name: {}", field.name));
            }
            if !bounds.min.is_finite()
                || !bounds.max.is_finite()
                || !bounds.min.cmplt(bounds.max).all()
            {
                return Err(format!("invalid bounds for {}", field.name));
            }
            if self.fields[..i]
                .iter()
                .any(|other| bounds.overlaps(other.bounds()))
            {
                return Err(format!("overlapping field: {}", field.name));
            }
            // A way's points reach the terrain grid through `Hollows`, and one
            // that is not a number sinks the ground to nothing — taking the
            // collider with it, far from anything that would name this layout.
            check_way(&field.name, &field.way, field.way_width, field.wear)?;
        }
        for (i, way) in self.ways.iter().enumerate() {
            if way.name.trim().is_empty()
                || self.ways[..i].iter().any(|other| other.name == way.name)
            {
                return Err(format!("empty or duplicate way name: {}", way.name));
            }
            if way.points.len() < 2 {
                return Err(format!("way {} needs two points to lead anywhere", way.name));
            }
            check_way(&way.name, &way.points, Some(way.width), Some(way.wear))?;
        }
        Ok(())
    }

    /// Row-major world-XZ friction samples, or None when no friction is authored.
    pub fn friction_samples(
        &self,
        grid: &super::HeightGrid,
        fallback: f64,
    ) -> Result<Option<Vec<f64>>, String> {
        if !fallback.is_finite()
            || fallback < 0.0
            || !grid.cell.is_finite()
            || grid.cell <= 0.0
            || grid.cols < 2
            || grid.rows < 2
        {
            return Err("invalid field friction grid or collider friction".into());
        }
        let domain = FieldBounds {
            min: Vec2::new(grid.min_x, grid.min_z),
            max: Vec2::new(
                grid.min_x + (grid.cols - 1) as f32 * grid.cell,
                grid.min_z + (grid.rows - 1) as f32 * grid.cell,
            ),
        };
        self.validate_bounds(domain)?;
        let count = grid.cols
            .checked_mul(grid.rows)
            .filter(|&n| n <= 16_777_216)
            .ok_or("field friction grid exceeds 16 million samples")?;
        if self.default_friction.is_none() && self.fields.iter().all(|field| field.friction.is_none()) {
            return Ok(None);
        }
        let default = self.default_friction.unwrap_or(fallback);
        let mut values = Vec::with_capacity(count);
        for row in 0..grid.rows {
            for col in 0..grid.cols {
                let point = Vec2::new(
                    grid.min_x + col as f32 * grid.cell,
                    grid.min_z + row as f32 * grid.cell,
                );
                let friction = self.fields
                    .iter()
                    .find(|field| {
                        point.cmpge(Vec2::from_array(field.min)).all()
                            && point.cmplt(Vec2::from_array(field.max)).all()
                    })
                    .and_then(|field| field.friction)
                    .unwrap_or(default);
                values.push(friction);
            }
        }
        Ok(Some(values))
    }

    /// The fields as they fall on `terrain`, cut to it, and the default cover
    /// over the rest of it. The terrain may lie anywhere over the layout.
    pub fn regions(&self, terrain: FieldBounds) -> Vec<FieldSpec> {
        let fields: Vec<FieldSpec> = self
            .fields
            .iter()
            .filter(|field| field.bounds().overlaps(terrain))
            .map(|field| FieldSpec {
                min: field.bounds().min.max(terrain.min).to_array(),
                max: field.bounds().max.min(terrain.max).to_array(),
                ..field.clone()
            })
            .collect();
        let mut xs = vec![terrain.min.x, terrain.max.x];
        let mut zs = vec![terrain.min.y, terrain.max.y];
        for field in &fields {
            xs.extend([field.min[0], field.max[0]]);
            zs.extend([field.min[1], field.max[1]]);
        }
        xs.sort_by(f32::total_cmp);
        zs.sort_by(f32::total_cmp);
        xs.dedup();
        zs.dedup();
        let mut regions = fields.clone();
        for z in zs.windows(2) {
            let mut start = None;
            for x in xs.windows(2) {
                let middle = Vec2::new((x[0] + x[1]) * 0.5, (z[0] + z[1]) * 0.5);
                let occupied = fields
                    .iter()
                    .any(|field| field.bounds().contains(middle));
                if occupied {
                    if let Some(left) = start.take() {
                        regions.push(self.background_region(left, x[0], z[0], z[1], regions.len()));
                    }
                } else {
                    start.get_or_insert(x[0]);
                }
            }
            if let Some(left) = start {
                regions.push(self.background_region(
                    left,
                    terrain.max.x,
                    z[0],
                    z[1],
                    regions.len(),
                ));
            }
        }
        regions
    }

    fn background_region(&self, x0: f32, x1: f32, z0: f32, z1: f32, index: usize) -> FieldSpec {
        FieldSpec {
            name: format!("__background/{index}"),
            profile: self.default.clone(),
            min: [x0, z0],
            max: [x1, z1],
            wear: None,
            way: Vec::new(),
            way_width: None,
            tyre: None,
            friction: self.default_friction,
        }
    }
}
