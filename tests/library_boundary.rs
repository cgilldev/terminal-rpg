use std::{fs, path::Path};
use terminal_rpg::{
    app::{AppMode, PlayOptions},
    game::{Command, Direction, RunSeed},
    session::{Intent, SkillSlot},
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
        debug_godmode: false,
    });
    let command = Command::Move(Direction::NorthWest);
    let position = Position::new(4, 7);

    assert!(matches!(mode, AppMode::Play(_)));
    assert_eq!(command, Command::Move(Direction::NorthWest));
    assert_eq!((position.x, position.y), (4, 7));
    assert_eq!(
        Intent::UseSkill(SkillSlot::CLEAVE),
        Intent::UseSkill(SkillSlot::new(1).unwrap())
    );
}

#[test]
fn domain_sources_do_not_import_adapter_dependencies() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let forbidden = [
        "crossterm",
        "ratatui",
        "russh",
        "axum",
        "tokio",
        "crate::app",
        "crate::server",
        "crate::ui",
        "crate::web",
        "std::net",
        "std::process",
    ];

    for boundary in ["game", "world"] {
        for entry in fs::read_dir(root.join(boundary)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }
            let source = fs::read_to_string(&path).unwrap();
            for dependency in forbidden {
                assert!(
                    !source.contains(dependency),
                    "{} must not depend on adapter concern {dependency}",
                    path.display()
                );
            }
            if boundary == "world" {
                assert!(
                    !source.contains("crate::game"),
                    "{} must not depend upward on game rules",
                    path.display()
                );
            }
        }
    }
}
