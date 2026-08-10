use terminal_rpg::{
    app::{AppMode, PlayOptions},
    game::{Command, Direction, RunSeed},
    ui::DisplayProfile,
    world::Position,
};

#[test]
fn library_types_can_describe_a_seeded_local_command() {
    let mode = AppMode::Play(PlayOptions {
        seed: Some(RunSeed(0x0DEC_0DED)),
        display: DisplayProfile {
            ascii: true,
            no_color: true,
        },
    });
    let command = Command::Move(Direction::NorthWest);
    let position = Position::new(4, 7);

    assert!(matches!(mode, AppMode::Play(_)));
    assert_eq!(command, Command::Move(Direction::NorthWest));
    assert_eq!((position.x, position.y), (4, 7));
}
