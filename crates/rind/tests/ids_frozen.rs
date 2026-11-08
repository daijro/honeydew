//! Rind's parameter ids are permanent: a saved song or preset names each by it. The table may only grow at its end
//! (`tests/ids.golden` holds the ids as released), and every id is unique, ASCII and the same shape.

use rind::ids::{COUNT, SPECS};

#[test]
fn the_released_ids_keep_their_places() {
    let golden: Vec<&str> = include_str!("ids.golden").lines().collect();
    assert!(SPECS.len() >= golden.len(), "an id was removed");
    for (i, id) in golden.iter().enumerate() {
        assert_eq!(SPECS[i].id, *id, "id {i} moved or was renamed");
    }
    assert_eq!(SPECS.len(), COUNT);
}

#[test]
fn ids_are_unique_and_plain() {
    let mut seen = std::collections::HashSet::new();
    for s in SPECS.iter() {
        assert!(seen.insert(s.id), "{} is twice in the table", s.id);
        assert!(
            !s.id.is_empty()
                && s.id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "{:?}",
            s.id
        );
    }
}

#[test]
fn every_default_is_inside_its_range_and_round_trips_through_normalised() {
    for (i, s) in SPECS.iter().enumerate() {
        let n = rind::ranges::to_norm(i, s.default);
        assert!((0.0..=1.0).contains(&n), "{}: {n}", s.id);
        let back = rind::ranges::from_norm(i, n);
        assert!(
            (back - s.default).abs() < 1e-3 * (1.0 + s.default.abs()),
            "{}: {back}",
            s.id
        );
    }
}
