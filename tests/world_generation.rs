use proptest::prelude::*;
use std::collections::HashSet;
use terminal_rpg::{
    game::RunSeed,
    world::{DungeonGenerator, GENERATOR_VERSION, Tile},
};

#[test]
fn one_thousand_seeded_dungeons_satisfy_invariants() {
    let generator = DungeonGenerator::default();
    for seed in 0..1_000 {
        let map = generator.generate(RunSeed(seed)).unwrap();
        map.validate().unwrap();
        assert_eq!(map.generator_version, GENERATOR_VERSION);
        assert!(map.spawn_candidates.len() >= 15);
        assert_eq!(
            map.spawn_candidates.iter().collect::<HashSet<_>>().len(),
            map.spawn_candidates.len()
        );
    }
}

#[test]
fn same_seed_produces_identical_map() {
    let generator = DungeonGenerator::default();
    assert_eq!(
        generator.generate(RunSeed(0xD4_4B)).unwrap(),
        generator.generate(RunSeed(0xD4_4B)).unwrap()
    );
}

#[test]
fn representative_floor_is_reviewable() {
    let map = DungeonGenerator::default()
        .generate(RunSeed(0xD4_4B))
        .unwrap();
    insta::assert_snapshot!(map.to_ascii());
}

proptest! {
    #[test]
    fn arbitrary_seeds_never_escape_bounds_or_disconnect(seed in any::<u64>()) {
        let map = DungeonGenerator::default().generate(RunSeed(seed)).unwrap();
        prop_assert!(map.contains(map.player_start));
        prop_assert!(map.contains(map.exit));
        prop_assert_eq!(map.tile(map.exit), Some(Tile::Exit));
        prop_assert!(map.traversable_positions().contains(&map.exit));
        prop_assert!(map.spawn_candidates.iter().all(|p| map.contains(*p)));
    }
}
