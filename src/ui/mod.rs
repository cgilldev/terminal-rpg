//! Shared Ratatui game presentation and local-terminal adapter.

mod local;

#[cfg(test)]
use crate::game::{Direction, ItemId, RunSeed};
pub use crate::session::{
    InputDecoder, Intent, MAX_INPUT_BYTES_PER_FEED, MAX_INTENTS_PER_FEED, SkillSlot,
    apply_game_intent,
};
use crate::{
    game::{
        AbilitySlot, ActorKind, ExplorationGame, InspectionVisibility, ItemEffect, RunStatus,
        Telegraph,
    },
    world::{Position, Tile},
};
#[cfg(test)]
use local::LocalTerminalGuard;
pub use local::run_local;
use ratatui::{
    Frame,
    layout::{Constraint, Direction as LayoutDirection, Layout, Rect, Size},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};

pub const MIN_WIDTH: u16 = 40;
pub const MIN_HEIGHT: u16 = 12;
pub const MAX_WIDTH: u16 = 300;
pub const MAX_HEIGHT: u16 = 120;

const ASCII_BORDER: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DisplayProfile {
    pub ascii: bool,
    pub no_color: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SemanticTone {
    Plain,
    Border,
    Player,
    Wall,
    Floor,
    Memory,
    Door,
    Exit,
    Skeleton,
    Ghoul,
    Cultist,
    Lunge,
    Hex,
    Health,
    Danger,
    Ready,
    Cooldown,
    Event,
    Help,
    Victory,
    Death,
    Title,
    Muted,
    Target,
    Candidate,
    Range,
    Item,
}

fn semantic_style(profile: DisplayProfile, tone: SemanticTone) -> Style {
    let modifier = match tone {
        SemanticTone::Player
        | SemanticTone::Exit
        | SemanticTone::Lunge
        | SemanticTone::Hex
        | SemanticTone::Danger
        | SemanticTone::Victory
        | SemanticTone::Death
        | SemanticTone::Title => Modifier::BOLD,
        SemanticTone::Memory | SemanticTone::Range => Modifier::DIM,
        SemanticTone::Target => Modifier::REVERSED | Modifier::BOLD,
        SemanticTone::Candidate => Modifier::UNDERLINED,
        _ => Modifier::empty(),
    };
    let style = Style::default().add_modifier(modifier);
    if profile.no_color {
        return style;
    }
    let color = match tone {
        SemanticTone::Plain => return style,
        SemanticTone::Border
        | SemanticTone::Wall
        | SemanticTone::Memory
        | SemanticTone::Cooldown
        | SemanticTone::Muted => Color::DarkGray,
        SemanticTone::Player | SemanticTone::Skeleton => Color::White,
        SemanticTone::Floor | SemanticTone::Event => Color::Gray,
        SemanticTone::Door | SemanticTone::Ready | SemanticTone::Title | SemanticTone::Item => {
            Color::Yellow
        }
        SemanticTone::Exit | SemanticTone::Victory => Color::LightYellow,
        SemanticTone::Ghoul | SemanticTone::Health => Color::Green,
        SemanticTone::Cultist => Color::Magenta,
        SemanticTone::Lunge | SemanticTone::Danger | SemanticTone::Death => Color::LightRed,
        SemanticTone::Hex => Color::LightMagenta,
        SemanticTone::Help
        | SemanticTone::Target
        | SemanticTone::Candidate
        | SemanticTone::Range => Color::LightCyan,
    };
    style.fg(color)
}

pub fn draw_game(frame: &mut Frame<'_>, game: &ExplorationGame, profile: DisplayProfile) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        draw_small_terminal(frame, area, profile);
        return;
    }
    if game.status == RunStatus::Title {
        draw_title(frame, area, game, profile);
        return;
    }

    let (map_area, status_area, events_area, footer_area) = active_layout(area);
    let map_block = panel(" The Ossuary ", profile);
    let map_inner = map_block.inner(map_area);
    let map_lines = exploration_map_lines(game, map_inner.width, map_inner.height, profile);
    frame.render_widget(Paragraph::new(map_lines).block(map_block), map_area);

    let (panel_title, status) = if game.targeting.is_some() {
        (" Target ", targeting_lines(game, profile))
    } else if game.inspecting.is_some() {
        (" Inspect ", inspection_lines(game, profile))
    } else if game.using_item {
        (" Use Item ", item_use_lines(game, profile))
    } else {
        (" Status ", status_lines(game, profile))
    };
    frame.render_widget(
        Paragraph::new(status)
            .block(panel(panel_title, profile))
            .wrap(Wrap { trim: true }),
        status_area,
    );
    let events = game
        .events
        .iter()
        .rev()
        .take(2)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    frame.render_widget(
        Paragraph::new(events)
            .style(semantic_style(profile, SemanticTone::Event))
            .block(panel(" Events ", profile)),
        events_area,
    );
    frame.render_widget(
        Paragraph::new(if game.targeting.is_some() {
            "TARGET Move keys | Tab cycle | Enter fire | Esc/2 cancel | Q quit"
        } else if game.inspecting.is_some() {
            "INSPECT Move keys | i/Esc exit | ?:Help r:Restart Q:Quit"
        } else if game.using_item {
            "USE ITEM 1-4 select | u/Esc cancel | ?:Help Q:Quit"
        } else {
            "Move qwe/asd/zxc | 1-0 Skills | g:Get u:Use i:Inspect ?:Help Q:Quit"
        })
        .style(semantic_style(profile, SemanticTone::Muted)),
        footer_area,
    );

    if game.character_info {
        let lines = character_info_lines(game, profile);
        let popup = centered_popup(area, &lines);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(lines)
                .block(panel(" Character ", profile))
                .wrap(Wrap { trim: true }),
            popup,
        );
        return;
    }
    if game.help {
        let popup = Rect::new(area.width / 2 - 26, area.height / 2 - 6, 52, 12);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(if game.targeting.is_some() {
                "TARGETING\nMove qwe/asd/zxc or arrows.\nTab / Shift-Tab cycles valid foes.\nEnter fires Grave Bolt. Esc or 2 cancels.\nSelection and cancellation use no turn.\n\n?: close help | Q: quit"
            } else if game.inspecting.is_some() {
                "INSPECTING\nMove qwe/asd/zxc or arrows.\nVisible tiles reveal current entities and omens.\nRemembered tiles reveal terrain only; unknown tiles reveal nothing.\ni or Esc exits. Inspection uses no turn.\n\n?: close help | Q: quit"
            } else if game.using_item {
                "USING ITEMS\nPress 1-4 for an inventory slot.\nPotions heal; torches are passive and remain carried.\nu or Esc cancels without using a turn.\n\n?: close help | Q: quit"
            } else {
                "Reach >. Move qwe/asd/zxc or arrows; s waits.\nBump + doors. s Skeleton, g Ghoul, c Cultist.\n! lunge, * hex. Movement and waiting use turns.\n\ng picks up; u then 1-4 uses an item.\n?: close | r: restart | Q: quit"
            })
            .style(semantic_style(profile, SemanticTone::Help))
            .block(panel(" Help ", profile))
                .wrap(Wrap { trim: true }),
            popup,
        );
    }
    draw_run_outcome(frame, area, game, profile);
}

fn active_layout(area: Rect) -> (Rect, Rect, Rect, Rect) {
    if area.height > area.width {
        let sections = Layout::default()
            .direction(LayoutDirection::Vertical)
            .constraints([
                Constraint::Min(17),
                Constraint::Length(4),
                Constraint::Length(14),
                Constraint::Length(1),
            ])
            .split(area);
        return (sections[0], sections[2], sections[1], sections[3]);
    }
    let sections = Layout::default()
        .direction(LayoutDirection::Vertical)
        .constraints([
            Constraint::Min(17),
            Constraint::Length(4),
            Constraint::Length(1),
        ])
        .split(area);
    let top = Layout::default()
        .direction(LayoutDirection::Horizontal)
        .constraints([Constraint::Min(52), Constraint::Length(26)])
        .split(sections[0]);
    (top[0], top[1], sections[1], sections[2])
}

fn resource_bar_line(
    label: &str,
    current: i32,
    maximum: i32,
    profile: DisplayProfile,
    tone: SemanticTone,
) -> Line<'static> {
    let maximum = maximum.max(0);
    let current = current.clamp(0, maximum);
    let filled = if maximum == 0 {
        0
    } else {
        usize::try_from(((i64::from(current) * 10) / i64::from(maximum)).clamp(0, 10))
            .unwrap_or_default()
    };
    let (full, empty, left, right) = if profile.ascii {
        ("#", "-", "[", "]")
    } else {
        ("█", "░", "", "")
    };
    let bar = format!(
        "{left}{}{empty}{right}",
        full.repeat(filled),
        empty = empty.repeat(10 - filled)
    );
    Line::styled(
        format!("{label} {bar} {current}/{maximum}"),
        semantic_style(profile, tone),
    )
}

#[allow(clippy::too_many_lines)]
fn character_info_lines(game: &ExplorationGame, profile: DisplayProfile) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled(
            game.class_definition().name.clone(),
            semantic_style(profile, SemanticTone::Player),
        ),
        resource_bar_line(
            "HP",
            game.player.health,
            game.player.max_health,
            profile,
            SemanticTone::Health,
        ),
        resource_bar_line(
            "MP",
            game.player.mana,
            game.player.max_mana,
            profile,
            SemanticTone::Candidate,
        ),
    ];
    lines.push(Line::from("Abilities:"));
    for number in 1..=10 {
        let slot = AbilitySlot::new(number).unwrap();
        if let Some(state) = game.ability_state(slot) {
            if let Some(def) = game.ability_definition(state.ability_id) {
                lines.push(Line::from(format!(
                    "{} {} — {} (M{} C{})",
                    number, def.name, def.description, def.mana_cost, state.cooldown_remaining
                )));
            }
        } else {
            lines.push(Line::styled(
                format!("{number} Empty"),
                semantic_style(profile, SemanticTone::Muted),
            ));
        }
    }
    lines.push(Line::from("Items:"));
    for (i, item) in game.inventory.iter().enumerate() {
        let text = item
            .and_then(|x| game.item_definition(x.item_id))
            .map_or_else(
                || format!("{} Empty", i + 1),
                |d| format!("{} {} — {}", i + 1, d.name, d.description),
            );
        lines.push(Line::from(text));
    }
    lines.push(Line::from("Tab closes"));
    lines
}

#[allow(clippy::too_many_lines)]
fn status_lines(game: &ExplorationGame, profile: DisplayProfile) -> Vec<Line<'static>> {
    let health_tone = if i64::from(game.player.health) * 3 <= i64::from(game.player.max_health) {
        SemanticTone::Danger
    } else {
        SemanticTone::Health
    };
    let mut lines = vec![
        Line::from(Span::styled(
            game.class_definition().name.to_uppercase(),
            semantic_style(profile, SemanticTone::Player),
        )),
        if game.godmode {
            Line::styled("GODMODE", semantic_style(profile, SemanticTone::Danger))
        } else {
            Line::from("")
        },
        resource_bar_line(
            "HP",
            game.player.health,
            game.player.max_health,
            profile,
            health_tone,
        ),
        resource_bar_line(
            "MP",
            game.player.mana,
            game.player.max_mana,
            profile,
            SemanticTone::Candidate,
        ),
        Line::styled(
            format!("Armor {}", game.player.armor),
            semantic_style(profile, SemanticTone::Floor),
        ),
    ];
    let mut empty = Vec::new();
    for number in 1..=10 {
        let slot = AbilitySlot::new(number).expect("status iterates valid slots");
        let key = if number == 10 {
            '0'
        } else {
            char::from(b'0' + number)
        };
        if let Some(state) = game.ability_state(slot) {
            let definition = game
                .ability_definition(state.ability_id)
                .expect("equipped abilities come from the run catalog");
            let (label, tone) = if state.cooldown_remaining != 0 {
                (
                    format!(
                        "{key} {} M{} C{}",
                        definition.name, definition.mana_cost, state.cooldown_remaining
                    ),
                    SemanticTone::Cooldown,
                )
            } else if game.can_afford_ability(slot) {
                (
                    format!("{key} {} M{} Ready", definition.name, definition.mana_cost),
                    SemanticTone::Ready,
                )
            } else {
                (
                    format!("{key} {} M{} Low", definition.name, definition.mana_cost),
                    SemanticTone::Danger,
                )
            };
            lines.push(Line::styled(label, semantic_style(profile, tone)));
        } else {
            empty.push(key);
        }
    }
    for keys in empty.chunks(2) {
        lines.push(Line::styled(
            format!(
                "{} Empty  {} Empty",
                keys[0],
                keys.get(1).copied().unwrap_or(' ')
            ),
            semantic_style(profile, SemanticTone::Muted),
        ));
    }
    let inventory = game
        .inventory
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let label = item
                .and_then(|item| game.item_definition(item.item_id))
                .map_or("Empty".to_owned(), |definition| definition.name.clone());
            format!("I{} {}", index + 1, label)
        })
        .collect::<Vec<_>>();
    lines.push(Line::styled(
        format!("{}  {}", inventory[0], inventory[1]),
        semantic_style(profile, SemanticTone::Item),
    ));
    lines.push(Line::styled(
        format!("{}  {}", inventory[2], inventory[3]),
        semantic_style(profile, SemanticTone::Item),
    ));
    lines.push(Line::styled(
        format!("T{}  Seed {}", game.turn, game.seed().0),
        semantic_style(profile, SemanticTone::Muted),
    ));
    lines.push(Line::styled(
        format!("Foes {}  {}", game.hostiles.len(), threat_summary(game)),
        semantic_style(
            profile,
            if game.hostiles.iter().any(|actor| actor.telegraph.is_some()) {
                SemanticTone::Danger
            } else {
                SemanticTone::Muted
            },
        ),
    ));
    lines
}

fn item_use_lines(game: &ExplorationGame, profile: DisplayProfile) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled("USE ITEM", semantic_style(profile, SemanticTone::Item)),
        Line::from("Choose slot 1-4"),
        Line::from(""),
    ];
    for (index, item) in game.inventory.iter().enumerate() {
        match item.and_then(|item| game.item_definition(item.item_id)) {
            Some(definition) => {
                let state = match definition.effect {
                    ItemEffect::TorchVision { .. } => "Passive",
                    ItemEffect::Heal { .. } => "Usable",
                    ItemEffect::Unsupported(_) => "Unavailable",
                };
                lines.push(Line::styled(
                    format!("{} {}", index + 1, definition.name),
                    semantic_style(profile, SemanticTone::Item),
                ));
                lines.push(Line::from(state));
            }
            None => lines.push(Line::styled(
                format!("{} Empty", index + 1),
                semantic_style(profile, SemanticTone::Muted),
            )),
        }
    }
    lines.extend([Line::from(""), Line::from("u / Esc cancels")]);
    lines
}

fn targeting_lines(game: &ExplorationGame, profile: DisplayProfile) -> Vec<Line<'static>> {
    let targeting = game
        .targeting
        .expect("targeting panel requires targeting state");
    let state = game
        .ability_state(targeting.ability_slot)
        .expect("targeted ability equipped");
    let definition = game
        .ability_definition(state.ability_id)
        .expect("targeted ability defined");
    let validity = match game.target_validity(targeting.ability_slot, targeting.cursor) {
        crate::game::TargetValidity::Valid(id) => format!("Valid foe #{}", id.0),
        crate::game::TargetValidity::NoHostile => "No hostile".into(),
        crate::game::TargetValidity::OutOfRange => "Out of range".into(),
        crate::game::TargetValidity::Blocked => "Line blocked".into(),
    };
    vec![
        Line::styled("TARGETING", semantic_style(profile, SemanticTone::Target)),
        Line::from(definition.name.clone()),
        Line::from(""),
        Line::from(format!(
            "Cursor {},{}",
            targeting.cursor.x, targeting.cursor.y
        )),
        Line::styled(validity, semantic_style(profile, SemanticTone::Ready)),
        Line::from(""),
        Line::from("Move qwe/asd/zxc"),
        Line::from("or arrow keys"),
        Line::from("Tab / Shift-Tab"),
        Line::from("cycle targets"),
        Line::from("Enter confirms"),
        Line::from("Esc or skill cancels"),
    ]
}

fn inspection_lines(game: &ExplorationGame, profile: DisplayProfile) -> Vec<Line<'static>> {
    let inspecting = game
        .inspecting
        .expect("inspect panel requires inspect state");
    let inspection = game.inspect(inspecting.cursor);
    let visibility = match inspection.visibility {
        InspectionVisibility::Unknown => "UNKNOWN",
        InspectionVisibility::Remembered => "REMEMBERED",
        InspectionVisibility::Visible => "VISIBLE",
    };
    let mut lines = vec![
        Line::styled("INSPECT", semantic_style(profile, SemanticTone::Target)),
        Line::from(format!(
            "{},{}  {visibility}",
            inspecting.cursor.x, inspecting.cursor.y
        )),
        Line::from(""),
    ];
    if let Some(terrain) = inspection.terrain {
        lines.push(Line::styled(
            terrain.name,
            semantic_style(profile, SemanticTone::Floor),
        ));
        lines.push(Line::from(terrain.description));
    } else {
        lines.push(Line::styled(
            "The dark keeps its secrets.",
            semantic_style(profile, SemanticTone::Muted),
        ));
    }
    for entity in inspection.entities {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            format!("{} #{}", entity.name, entity.actor_id.0),
            semantic_style(profile, SemanticTone::Player),
        ));
        lines.push(Line::from(format!(
            "HP {}/{}  Armor {}  {}",
            entity.health,
            entity.max_health,
            entity.armor,
            if entity.active { "Active" } else { "Dormant" }
        )));
        lines.push(Line::from(entity.description));
    }
    for marker in inspection.markers {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            marker.name,
            semantic_style(profile, SemanticTone::Danger),
        ));
        lines.push(Line::from(marker.description));
    }
    for item in inspection.items {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            item.name,
            semantic_style(profile, SemanticTone::Item),
        ));
        lines.push(Line::from(item.description));
    }
    for item in inspection.carried_items {
        lines.push(Line::from(""));
        lines.push(Line::styled(
            item.name,
            semantic_style(profile, SemanticTone::Item),
        ));
        lines.push(Line::from(item.description));
    }
    lines
}

fn draw_run_outcome(
    frame: &mut Frame<'_>,
    area: Rect,
    game: &ExplorationGame,
    profile: DisplayProfile,
) {
    if game.status == RunStatus::Victory {
        let popup = Rect::new(area.width / 2 - 22, area.height / 2 - 4, 44, 8);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(format!(
                "YOU ESCAPED THE OSSUARY\n\nThe {} returns to the dying light.\n\nPress r for another descent or Q to quit.",
                game.class_definition().name
            ))
            .style(semantic_style(profile, SemanticTone::Victory))
            .block(panel(" Victory ", profile))
            .wrap(Wrap { trim: true }),
            popup,
        );
    } else if game.status == RunStatus::Death {
        let popup = Rect::new(area.width / 2 - 22, area.height / 2 - 4, 44, 8);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(format!(
                "YOU DIED\n\nThe Ossuary claims the {}.\n\nPress r for another descent or Q to quit.",
                game.class_definition().name
            ))
            .style(semantic_style(profile, SemanticTone::Death))
            .block(panel(" Death ", profile))
            .wrap(Wrap { trim: true }),
            popup,
        );
    }
}

fn draw_title(frame: &mut Frame<'_>, area: Rect, game: &ExplorationGame, profile: DisplayProfile) {
    let vertical = Layout::vertical([
        Constraint::Percentage(35),
        Constraint::Length(9),
        Constraint::Percentage(35),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("TERMINAL RPG", semantic_style(profile, SemanticTone::Title)),
            Line::from(""),
            Line::styled(
                "A dark-fantasy descent into the Ossuary.",
                semantic_style(profile, SemanticTone::Event),
            ),
            Line::from(""),
            Line::styled(
                format!(
                    "Seed {}  Generator v{}",
                    game.seed().0,
                    game.generator_version()
                ),
                semantic_style(profile, SemanticTone::Muted),
            ),
            Line::from(""),
            Line::styled(
                "Press Enter to descend. Press Q to quit.",
                semantic_style(profile, SemanticTone::Ready),
            ),
        ])
        .block(panel(
            format!(" {} ", game.class_definition().name),
            profile,
        ))
        .wrap(Wrap { trim: true }),
        vertical[1],
    );
}

fn draw_small_terminal(frame: &mut Frame<'_>, area: Rect, profile: DisplayProfile) {
    frame.render_widget(
        Paragraph::new(format!(
            "Terminal too small: {}x{}. Resize to at least {MIN_WIDTH}x{MIN_HEIGHT}. No turns pass while this message is shown.",
            area.width, area.height
        ))
        .style(semantic_style(profile, SemanticTone::Danger))
        .block(panel(" Terminal RPG ", profile))
        .wrap(Wrap { trim: true }),
        area,
    );
}

fn exploration_map_lines(
    game: &ExplorationGame,
    width: u16,
    height: u16,
    profile: DisplayProfile,
) -> Vec<Line<'static>> {
    let view_width = i32::from(width);
    let view_height = i32::from(height);
    let max_x = (i32::from(game.map.width()) - view_width).max(0);
    let max_y = (i32::from(game.map.height()) - view_height).max(0);
    let focus = game
        .inspecting
        .map(|state| state.cursor)
        .or_else(|| game.targeting.map(|state| state.cursor))
        .unwrap_or(game.player.position);
    let origin_x = (focus.x - view_width / 2).clamp(0, max_x);
    let origin_y = (focus.y - view_height / 2).clamp(0, max_y);
    (0..view_height)
        .map(|screen_y| {
            let mut spans = Vec::with_capacity(usize::from(width));
            for screen_x in 0..view_width {
                let position = Position::new(origin_x + screen_x, origin_y + screen_y);
                let (glyph, tone) = exploration_cell(game, position, profile);
                spans.push(Span::styled(
                    glyph.to_string(),
                    semantic_style(profile, tone),
                ));
            }
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
fn exploration_glyph(game: &ExplorationGame, position: Position, profile: DisplayProfile) -> char {
    exploration_cell(game, position, profile).0
}

fn exploration_cell(
    game: &ExplorationGame,
    position: Position,
    profile: DisplayProfile,
) -> (char, SemanticTone) {
    if game
        .inspecting
        .is_some_and(|state| state.cursor == position)
    {
        let (glyph, _) = exploration_base_cell(game, position, profile);
        return (glyph, SemanticTone::Target);
    }
    if let Some(targeting) = game.targeting {
        if position == targeting.cursor {
            let (glyph, _) = exploration_base_cell(game, position, profile);
            return (glyph, SemanticTone::Target);
        }
        if matches!(
            game.target_validity(targeting.ability_slot, position),
            crate::game::TargetValidity::Valid(_)
        ) {
            let (glyph, _) = exploration_base_cell(game, position, profile);
            return (glyph, SemanticTone::Candidate);
        }
        if game.is_visible(position) {
            let state = game
                .ability_state(targeting.ability_slot)
                .expect("targeted ability is equipped");
            let definition = game
                .ability_definition(state.ability_id)
                .expect("targeted ability is defined");
            if matches!(
                definition.targeting,
                crate::game::AbilityTargeting::HostileSingle { range, .. }
                    if game.player.position.chebyshev_distance(position) == range
            ) {
                let (glyph, _) = exploration_base_cell(game, position, profile);
                return (glyph, SemanticTone::Range);
            }
        }
    }
    exploration_base_cell(game, position, profile)
}

fn exploration_base_cell(
    game: &ExplorationGame,
    position: Position,
    profile: DisplayProfile,
) -> (char, SemanticTone) {
    if game.is_visible(position) {
        if position == game.player.position {
            return ('@', SemanticTone::Player);
        }
        if let Some(telegraph) = game.telegraph_at(position) {
            return match telegraph {
                Telegraph::GhoulLunge { .. } => ('!', SemanticTone::Lunge),
                Telegraph::CultistHex { .. } => ('*', SemanticTone::Hex),
            };
        }
        if let Some(actor) = game
            .hostiles
            .iter()
            .find(|actor| actor.position == position)
        {
            return match (actor.kind, actor.telegraph.is_some()) {
                (ActorKind::Skeleton, _) => ('s', SemanticTone::Skeleton),
                (ActorKind::Ghoul, false) => ('g', SemanticTone::Ghoul),
                (ActorKind::Ghoul, true) => ('G', SemanticTone::Lunge),
                (ActorKind::Cultist, false) => ('c', SemanticTone::Cultist),
                (ActorKind::Cultist, true) => ('C', SemanticTone::Hex),
            };
        }
        if let Some(ground) = game
            .ground_items
            .iter()
            .find(|ground| ground.position == position)
        {
            return {
                let definition = game
                    .item_definition(ground.item.item_id)
                    .expect("ground item definition");
                (
                    if profile.ascii {
                        definition.glyph_ascii
                    } else {
                        definition.glyph_unicode
                    },
                    SemanticTone::Item,
                )
            };
        }
        return match game.map.tile(position) {
            Some(Tile::Wall) if profile.ascii => ('#', SemanticTone::Wall),
            Some(Tile::Wall) => ('█', SemanticTone::Wall),
            Some(Tile::Floor) if profile.ascii => ('.', SemanticTone::Floor),
            Some(Tile::Floor) => ('·', SemanticTone::Floor),
            Some(Tile::ClosedDoor) if profile.ascii => ('+', SemanticTone::Door),
            Some(Tile::ClosedDoor) => ('╬', SemanticTone::Door),
            Some(Tile::OpenDoor) if profile.ascii => ('/', SemanticTone::Door),
            Some(Tile::OpenDoor) => ('╱', SemanticTone::Door),
            Some(Tile::Exit) if profile.ascii => ('>', SemanticTone::Exit),
            Some(Tile::Exit) => ('▾', SemanticTone::Exit),
            None => (' ', SemanticTone::Plain),
        };
    }
    if game.is_explored(position) {
        return match game.map.tile(position) {
            Some(Tile::Wall) if profile.ascii => ('%', SemanticTone::Memory),
            Some(Tile::Wall) => ('░', SemanticTone::Memory),
            Some(_) => (',', SemanticTone::Memory),
            None => (' ', SemanticTone::Plain),
        };
    }
    (' ', SemanticTone::Plain)
}

fn threat_summary(game: &ExplorationGame) -> String {
    let lunges = game
        .hostiles
        .iter()
        .filter(|actor| {
            matches!(
                actor.telegraph,
                Some(crate::game::Telegraph::GhoulLunge { .. })
            )
        })
        .count();
    let hexes = game
        .hostiles
        .iter()
        .filter(|actor| {
            matches!(
                actor.telegraph,
                Some(crate::game::Telegraph::CultistHex { .. })
            )
        })
        .count();
    if lunges == 0 && hexes == 0 {
        "Threat  None".into()
    } else {
        format!("Threat  !{lunges} *{hexes} on @")
    }
}

#[must_use]
pub fn capped_area(width: u32, height: u32) -> Rect {
    Rect::new(
        0,
        0,
        u16::try_from(width)
            .unwrap_or(MAX_WIDTH)
            .clamp(1, MAX_WIDTH),
        u16::try_from(height)
            .unwrap_or(MAX_HEIGHT)
            .clamp(1, MAX_HEIGHT),
    )
}

#[must_use]
pub const fn intent_allowed_at_size(intent: Intent, area: Size) -> bool {
    matches!(intent, Intent::Quit) || (area.width >= MIN_WIDTH && area.height >= MIN_HEIGHT)
}

fn panel(title: impl Into<Line<'static>>, profile: DisplayProfile) -> Block<'static> {
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(semantic_style(profile, SemanticTone::Border));
    if profile.ascii {
        block.border_set(ASCII_BORDER)
    } else {
        block
    }
}

/// A centered popup `Rect` sized to fit `lines` plus a border, clamped to
/// `area` so it never overflows the frame at the minimum supported size.
fn centered_popup(area: Rect, lines: &[Line<'static>]) -> Rect {
    let content_width = lines
        .iter()
        .map(|line| u16::try_from(line.width()).unwrap_or_default())
        .max()
        .unwrap_or_default();
    let popup_width = content_width
        .saturating_add(2)
        .clamp(2, area.width.saturating_sub(1));
    let content_height = u16::try_from(lines.len()).unwrap_or_default();
    let popup_height = content_height
        .saturating_add(2)
        .clamp(2, area.height.saturating_sub(1));
    Rect::new(
        area.x + (area.width.saturating_sub(popup_width)) / 2,
        area.y + (area.height.saturating_sub(popup_height)) / 2,
        popup_width,
        popup_height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::io::{self, Write};

    fn rendered_game(profile: DisplayProfile, status: RunStatus) -> String {
        rendered_game_at(80, 24, profile, status, false)
    }

    fn rendered_game_at(
        width: u16,
        height: u16,
        profile: DisplayProfile,
        status: RunStatus,
        help: bool,
    ) -> String {
        let terminal = game_terminal_at(width, height, profile, status, help);
        terminal_text(&terminal, width, height)
    }

    fn game_terminal_at(
        width: u16,
        height: u16,
        profile: DisplayProfile,
        status: RunStatus,
        help: bool,
    ) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut game = ExplorationGame::new(Some(RunSeed(0xD4_4B))).unwrap();
        game.start();
        game.status = status;
        game.help = help;
        if status == RunStatus::Death {
            game.player.health = 0;
            game.events.push("The Grave Knight falls.".into());
        }
        terminal
            .draw(|frame| draw_game(frame, &game, profile))
            .unwrap();
        terminal
    }

    fn character_terminal(profile: DisplayProfile) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut game = ExplorationGame::new(Some(RunSeed(0xD4_4B))).unwrap();
        game.start();
        game.character_info = true;
        terminal
            .draw(|frame| draw_game(frame, &game, profile))
            .unwrap();
        terminal
    }

    fn alternate_class_terminal(status: RunStatus) -> Terminal<TestBackend> {
        use crate::game::{
            AbilityAvailability, AbilityBinding, AbilityDefinition, AbilityEffect, AbilityId,
            AbilityTargeting, ClassDefinition, ClassId, GameCatalog,
        };

        let class_id = ClassId(99);
        let ability_id = AbilityId(77);
        let catalog = GameCatalog::new(
            vec![AbilityDefinition {
                id: ability_id,
                name: "Test Sweep".into(),
                description: "Sweep nearby foes.".into(),
                mana_cost: 1,
                cooldown_turns: 2,
                targeting: AbilityTargeting::Immediate,
                effect: AbilityEffect::Cleave,
                availability: AbilityAvailability::Class(class_id),
            }],
            vec![ClassDefinition {
                id: class_id,
                name: "Test Warden".into(),
                description: "A sentinel raised against the dark.".into(),
                max_health: i32::MAX,
                max_mana: 10,
                mana_regeneration: 1,
                armor: i32::MAX,
                base_damage: i32::MAX,
                starting_abilities: vec![AbilityBinding {
                    slot: 3,
                    ability_id,
                }],
            }],
        )
        .unwrap();
        let mut game =
            ExplorationGame::new_with_catalog(Some(RunSeed(0xA17C_1A55)), class_id, catalog)
                .unwrap();
        if status != RunStatus::Title {
            game.start();
            game.status = status;
        }
        if status == RunStatus::Death {
            game.player.health = 0;
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| draw_game(frame, &game, DisplayProfile::default()))
            .unwrap();
        terminal
    }

    #[test]
    fn alternate_class_identity_renders_in_every_run_state_without_overflow() {
        for status in [
            RunStatus::Title,
            RunStatus::Active,
            RunStatus::Victory,
            RunStatus::Death,
        ] {
            let terminal = alternate_class_terminal(status);
            let rendered = terminal_text(&terminal, 80, 24);
            assert!(rendered.contains("Test Warden") || rendered.contains("TEST WARDEN"));
            assert!(!rendered.contains("Grave Knight"));
        }
    }

    fn map_tile_glyphs(profile: DisplayProfile) -> Vec<&'static str> {
        let mut glyphs = if profile.ascii {
            vec!["#", ".", "+", "/", ">"]
        } else {
            vec!["█", "·", "╬", "╱", "▾"]
        };
        glyphs.extend(if profile.ascii {
            ["%", ","]
        } else {
            ["░", ","]
        });
        glyphs
    }

    #[test]
    fn character_menu_popup_hides_the_map_beneath_it() {
        let profiles = [
            DisplayProfile::default(),
            DisplayProfile {
                ascii: true,
                no_color: true,
            },
        ];
        for profile in profiles {
            let glyphs = map_tile_glyphs(profile);
            let normal = game_terminal_at(80, 24, profile, RunStatus::Active, false);
            let popup = character_terminal(profile);
            let normal_buffer = normal.backend().buffer();
            let popup_buffer = popup.backend().buffer();
            let mut game = ExplorationGame::new(Some(RunSeed(0xD4_4B))).unwrap();
            game.start();
            game.character_info = true;
            let lines = character_info_lines(&game, profile);
            let rect = centered_popup(Rect::new(0, 0, 80, 24), &lines);
            for y in rect.y..rect.y + rect.height {
                for x in rect.x..rect.x + rect.width {
                    let before = normal_buffer[(x, y)].symbol();
                    if glyphs.contains(&before) {
                        assert_ne!(
                            popup_buffer[(x, y)].symbol(),
                            before,
                            "map tile {before:?} shows through character popup at ({x},{y}) in {profile:?}"
                        );
                    }
                }
            }
            let rendered = terminal_text(&popup, 80, 24);
            assert!(rendered.contains("Abilities:"));
            assert!(rendered.contains("Tab closes"));
        }
    }

    #[test]
    fn character_menu_profiles_are_snapshot_tested() {
        let ascii = DisplayProfile {
            ascii: true,
            no_color: true,
        };
        insta::assert_snapshot!(
            "character_menu_unicode",
            terminal_text(&character_terminal(DisplayProfile::default()), 80, 24)
        );
        insta::assert_snapshot!(
            "character_menu_ascii",
            terminal_text(&character_terminal(ascii), 80, 24)
        );
    }

    fn rendered_enemy_showcase(profile: DisplayProfile) -> String {
        let terminal = enemy_showcase_terminal(profile);
        terminal_text(&terminal, 80, 24)
    }

    fn targeting_terminal(profile: DisplayProfile, mode: &str) -> Terminal<TestBackend> {
        let mut game = ExplorationGame::new(Some(RunSeed(0xD4_4B))).unwrap();
        game.start();
        game.hostiles.clear();
        let slot = AbilitySlot::new(2).unwrap();
        let valid = game
            .map
            .positions()
            .filter(|position| {
                game.map.tile(*position) == Some(Tile::Floor)
                    && game.player.position.chebyshev_distance(*position) > 0
                    && game.player.position.chebyshev_distance(*position) <= 6
                    && game.is_visible(*position)
            })
            .min()
            .expect("generated map has a visible Grave Bolt target tile");
        game.spawn_hostile(ActorKind::Skeleton, valid, 20, 1, 1);
        game.apply(crate::game::Command::UseAbility(slot));
        if mode == "help" {
            game.help = true;
        }
        if !matches!(mode, "valid" | "help") {
            let desired = game
                .map
                .positions()
                .filter(|position| game.map.tile(*position) == Some(Tile::Floor))
                .find(|position| match mode {
                    "blocked" => {
                        game.player.position.chebyshev_distance(*position) <= 6
                            && !game.is_visible(*position)
                    }
                    "out" => game.player.position.chebyshev_distance(*position) > 6,
                    _ => false,
                })
                .expect("generated map has requested invalid target tile");
            let id = game.spawn_hostile(ActorKind::Ghoul, desired, 20, 0, 1);
            let targeting = game.targeting.as_mut().unwrap();
            targeting.cursor = desired;
            targeting.selected_actor = Some(id);
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| draw_game(frame, &game, profile))
            .unwrap();
        terminal
    }

    fn inspection_terminal(profile: DisplayProfile, mode: &str) -> Terminal<TestBackend> {
        let mut game = ExplorationGame::new(Some(RunSeed(0xD4_4B))).unwrap();
        game.start();
        game.hostiles.clear();
        game.apply(crate::game::Command::ToggleInspect);
        match mode {
            "player" => {}
            "hostile" => {
                let position = game
                    .visible
                    .iter()
                    .copied()
                    .filter(|position| {
                        *position != game.player.position
                            && game.map.tile(*position) == Some(Tile::Floor)
                    })
                    .min()
                    .unwrap();
                game.spawn_hostile(ActorKind::Ghoul, position, 14, 1, 4);
                game.hostiles[0].active = true;
                game.hostiles[0].telegraph = Some(Telegraph::GhoulLunge {
                    target: game.player.position,
                });
                game.inspecting.as_mut().unwrap().cursor = position;
            }
            "remembered" => {
                let position = game
                    .map
                    .positions()
                    .find(|position| !game.is_visible(*position))
                    .unwrap();
                game.explored.insert(position);
                game.inspecting.as_mut().unwrap().cursor = position;
            }
            "unknown" => {
                let position = game
                    .map
                    .positions()
                    .find(|position| !game.is_explored(*position))
                    .unwrap();
                game.inspecting.as_mut().unwrap().cursor = position;
            }
            "help" => game.help = true,
            "edge" => {
                game.inspecting.as_mut().unwrap().cursor = Position::new(0, 0);
            }
            "details" => {
                let position = game.player.position;
                let first = game.spawn_hostile(ActorKind::Skeleton, position, 10, 2, 3);
                game.spawn_hostile(ActorKind::Cultist, position, 9, 0, 4);
                game.hostiles[0].active = true;
                game.hostiles[0].telegraph = Some(Telegraph::CultistHex { target: position });
                assert_eq!(game.hostiles[0].id, first);
            }
            "cultist" => {
                let position = game
                    .visible
                    .iter()
                    .copied()
                    .filter(|position| {
                        *position != game.player.position
                            && game.map.tile(*position) == Some(Tile::Floor)
                    })
                    .min()
                    .unwrap();
                game.spawn_hostile(ActorKind::Cultist, position, 9, 0, 4);
                game.hostiles[0].telegraph = Some(Telegraph::CultistHex {
                    target: game.player.position,
                });
                game.inspecting.as_mut().unwrap().cursor = position;
            }
            "hex_target" => {
                let mut positions = game
                    .visible
                    .iter()
                    .copied()
                    .filter(|position| {
                        *position != game.player.position
                            && game.map.tile(*position) == Some(Tile::Floor)
                    })
                    .collect::<Vec<_>>();
                positions.sort_unstable();
                let origin = positions[0];
                let target = positions[1];
                game.spawn_hostile(ActorKind::Cultist, origin, 9, 0, 4);
                game.hostiles[0].telegraph = Some(Telegraph::CultistHex { target });
                game.inspecting.as_mut().unwrap().cursor = target;
            }
            "wall" | "closed_door" | "open_door" | "exit" => {
                inspect_terrain_fixture(&mut game, mode);
            }
            _ => unreachable!(),
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| draw_game(frame, &game, profile))
            .unwrap();
        terminal
    }

    fn inspect_terrain_fixture(game: &mut ExplorationGame, mode: &str) {
        let tile = match mode {
            "wall" => Tile::Wall,
            "closed_door" => Tile::ClosedDoor,
            "open_door" => Tile::OpenDoor,
            "exit" => Tile::Exit,
            _ => unreachable!(),
        };
        let mut position = game
            .map
            .positions()
            .find(|position| game.map.tile(*position) == Some(tile));
        if position.is_none() && tile == Tile::OpenDoor {
            let door = game
                .map
                .positions()
                .find(|position| game.map.tile(*position) == Some(Tile::ClosedDoor))
                .unwrap();
            assert!(game.map.open_door(door));
            position = Some(door);
        }
        let position = position.unwrap();
        game.visible.insert(position);
        game.explored.insert(position);
        game.inspecting.as_mut().unwrap().cursor = position;
    }

    #[test]
    fn inspection_modes_are_snapshot_tested() {
        for (profile, suffix) in [
            (DisplayProfile::default(), "unicode"),
            (
                DisplayProfile {
                    ascii: false,
                    no_color: true,
                },
                "unicode_nocolor",
            ),
            (
                DisplayProfile {
                    ascii: true,
                    no_color: false,
                },
                "ascii_color",
            ),
            (
                DisplayProfile {
                    ascii: true,
                    no_color: true,
                },
                "ascii",
            ),
        ] {
            for mode in [
                "player",
                "hostile",
                "cultist",
                "hex_target",
                "remembered",
                "unknown",
                "help",
                "edge",
                "details",
                "wall",
                "closed_door",
                "open_door",
                "exit",
            ] {
                let terminal = inspection_terminal(profile, mode);
                insta::assert_snapshot!(
                    format!("inspection_{mode}_{suffix}"),
                    terminal_text(&terminal, 80, 24)
                );
            }
        }
    }

    fn item_terminal(profile: DisplayProfile, mode: &str) -> Terminal<TestBackend> {
        use crate::game::{ItemInstance, ItemInstanceId};
        let mut game = ExplorationGame::new(Some(RunSeed(0x17E0))).unwrap();
        game.start();
        game.hostiles.clear();
        game.ground_items.clear();
        let mut floors = game
            .visible
            .iter()
            .copied()
            .filter(|position| {
                *position != game.player.position && game.map.tile(*position) == Some(Tile::Floor)
            })
            .collect::<Vec<_>>();
        floors.sort_unstable();
        game.spawn_ground_item(ItemId::TORCH, floors[0]);
        game.spawn_ground_item(ItemId::HEALTH_POTION, floors[1]);
        game.inventory[0] = Some(ItemInstance {
            instance_id: ItemInstanceId(80),
            item_id: ItemId::TORCH,
        });
        game.inventory[2] = Some(ItemInstance {
            instance_id: ItemInstanceId(81),
            item_id: ItemId::HEALTH_POTION,
        });
        match mode {
            "ground" => {}
            "use" => game.using_item = true,
            "empty" => game.inventory = [None; crate::game::INVENTORY_SLOT_COUNT],
            "full" => {
                for (index, slot) in game.inventory.iter_mut().enumerate() {
                    *slot = Some(ItemInstance {
                        instance_id: ItemInstanceId(u32::try_from(90 + index).unwrap()),
                        item_id: if index % 2 == 0 {
                            ItemId::TORCH
                        } else {
                            ItemId::HEALTH_POTION
                        },
                    });
                }
            }
            "torch_view" => {
                game.recompute_visibility();
            }
            "rejection" => {
                game.apply(crate::game::Command::PickupItem);
            }
            "use_rejection" => {
                game.player.health = game.player.max_health;
                game.using_item = true;
                assert_eq!(
                    game.apply(crate::game::Command::UseItemSlot(3)),
                    crate::game::CommandOutcome::Rejected
                );
            }
            "item_unknown" | "item_remembered" => {
                let position = floors[1];
                game.visible.remove(&position);
                if mode == "item_unknown" {
                    game.explored.remove(&position);
                } else {
                    game.explored.insert(position);
                }
                game.apply(crate::game::Command::ToggleInspect);
                game.inspecting.as_mut().unwrap().cursor = position;
            }
            "inspect" => {
                game.apply(crate::game::Command::ToggleInspect);
                game.inspecting.as_mut().unwrap().cursor = floors[1];
            }
            _ => unreachable!(),
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| draw_game(frame, &game, profile))
            .unwrap();
        terminal
    }

    #[test]
    fn item_ground_inventory_use_and_inspection_are_snapshot_tested() {
        for (profile, suffix) in [
            (DisplayProfile::default(), "unicode"),
            (
                DisplayProfile {
                    ascii: false,
                    no_color: true,
                },
                "unicode_nocolor",
            ),
            (
                DisplayProfile {
                    ascii: true,
                    no_color: false,
                },
                "ascii_color",
            ),
            (
                DisplayProfile {
                    ascii: true,
                    no_color: true,
                },
                "ascii",
            ),
        ] {
            for mode in [
                "ground",
                "use",
                "inspect",
                "empty",
                "full",
                "torch_view",
                "rejection",
                "use_rejection",
                "item_unknown",
                "item_remembered",
            ] {
                let terminal = item_terminal(profile, mode);
                insta::assert_snapshot!(
                    format!("items_{mode}_{suffix}"),
                    terminal_text(&terminal, 80, 24)
                );
            }
        }
    }

    #[test]
    fn targeting_modes_are_snapshot_tested() {
        for (profile, suffix) in [
            (DisplayProfile::default(), "unicode"),
            (
                DisplayProfile {
                    ascii: true,
                    no_color: true,
                },
                "ascii",
            ),
        ] {
            for mode in ["valid", "blocked", "out", "help"] {
                let terminal = targeting_terminal(profile, mode);
                insta::assert_snapshot!(
                    format!("targeting_{mode}_{suffix}"),
                    terminal_text(&terminal, 80, 24)
                );
            }
        }
    }

    fn enemy_showcase_terminal(profile: DisplayProfile) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut game = ExplorationGame::new(Some(RunSeed(0xD4_4B))).unwrap();
        game.start();
        let mut positions = game
            .visible
            .iter()
            .copied()
            .filter(|position| {
                *position != game.player.position && game.map.tile(*position) == Some(Tile::Floor)
            })
            .collect::<Vec<_>>();
        positions.sort_unstable();
        assert!(positions.len() >= 3);
        game.hostiles.truncate(3);
        for (actor, (kind, position)) in game.hostiles.iter_mut().zip([
            (crate::game::ActorKind::Skeleton, positions[0]),
            (crate::game::ActorKind::Ghoul, positions[1]),
            (crate::game::ActorKind::Cultist, positions[2]),
        ]) {
            actor.kind = kind;
            actor.position = position;
            actor.active = true;
            actor.telegraph = None;
        }
        game.hostiles[1].telegraph = Some(crate::game::Telegraph::GhoulLunge {
            target: game.player.position,
        });
        game.hostiles[2].telegraph = Some(crate::game::Telegraph::CultistHex {
            target: game.player.position,
        });
        game.events.push("Ghoul 2 marks a lunge.".into());
        game.events.push("Cultist 3 marks a hex.".into());
        terminal
            .draw(|frame| draw_game(frame, &game, profile))
            .unwrap();
        terminal
    }

    fn terminal_text(terminal: &Terminal<TestBackend>, width: u16, height: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                let mut row = String::new();
                for x in 0..width {
                    row.push_str(buffer[(x, y)].symbol());
                }
                row.trim_end().to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn text_position(
        terminal: &Terminal<TestBackend>,
        width: u16,
        height: u16,
        text: &str,
    ) -> (u16, u16) {
        let buffer = terminal.backend().buffer();
        let symbols = text.chars().collect::<Vec<_>>();
        for y in 0..height {
            for x in 0..=width.saturating_sub(u16::try_from(symbols.len()).unwrap()) {
                if symbols.iter().enumerate().all(|(offset, symbol)| {
                    buffer[(x + u16::try_from(offset).unwrap(), y)].symbol() == symbol.to_string()
                }) {
                    return (x, y);
                }
            }
        }
        panic!("rendered buffer did not contain {text:?}");
    }

    fn assert_rendered_text_color(terminal: &Terminal<TestBackend>, text: &str, expected: Color) {
        let buffer = terminal.backend().buffer();
        let characters = text.chars().collect::<Vec<_>>();
        let text_width = u16::try_from(characters.len()).expect("test text fits terminal width");
        for y in 0..buffer.area.height {
            for x in 0..=buffer.area.width.saturating_sub(text_width) {
                let matches = (0..text_width).all(|offset| {
                    buffer[(x + offset, y)].symbol() == characters[usize::from(offset)].to_string()
                });
                if matches {
                    assert!(
                        (0..text_width).all(|offset| buffer[(x + offset, y)].fg == expected),
                        "{text:?} was not entirely rendered with {expected:?}"
                    );
                    return;
                }
            }
        }
        panic!("rendered buffer did not contain {text:?}");
    }

    #[test]
    fn decoder_maps_spatial_movement_arrows_skills_and_quit() {
        let mut decoder = InputDecoder::default();
        assert_eq!(
            decoder.feed(b"qwea"),
            [
                Intent::Move(Direction::NorthWest),
                Intent::Move(Direction::North),
                Intent::Move(Direction::NorthEast),
                Intent::Move(Direction::West),
            ]
        );
        assert_eq!(
            decoder.feed(b"sdzx"),
            [
                Intent::Wait,
                Intent::Move(Direction::East),
                Intent::Move(Direction::SouthWest),
                Intent::Move(Direction::South),
            ]
        );
        assert_eq!(decoder.feed(b"c"), [Intent::Move(Direction::SouthEast)]);

        assert!(decoder.feed(b"\x1b[").is_empty());
        assert_eq!(decoder.feed(b"A"), vec![Intent::Move(Direction::North)]);
        for (suffix, direction) in [
            (b'B', Direction::South),
            (b'C', Direction::East),
            (b'D', Direction::West),
        ] {
            assert!(decoder.feed(b"\x1b[").is_empty());
            assert_eq!(decoder.feed(&[suffix]), [Intent::Move(direction)]);
        }

        assert_eq!(
            decoder.feed(b"1234"),
            [
                Intent::UseSkill(SkillSlot::new(1).unwrap()),
                Intent::UseSkill(SkillSlot::new(2).unwrap()),
                Intent::UseSkill(SkillSlot::new(3).unwrap()),
                Intent::UseSkill(SkillSlot::new(4).unwrap()),
            ]
        );
        assert_eq!(
            decoder.feed(b"5678"),
            [
                Intent::UseSkill(SkillSlot::new(5).unwrap()),
                Intent::UseSkill(SkillSlot::new(6).unwrap()),
                Intent::UseSkill(SkillSlot::new(7).unwrap()),
                Intent::UseSkill(SkillSlot::new(8).unwrap()),
            ]
        );
        assert_eq!(
            decoder.feed(b"90"),
            [
                Intent::UseSkill(SkillSlot::new(9).unwrap()),
                Intent::UseSkill(SkillSlot::new(10).unwrap()),
            ]
        );
        assert_eq!(decoder.feed(b"Q"), [Intent::Quit]);
        assert!(decoder.feed(b"hjkl ybn.").is_empty());
        assert_eq!(
            decoder.feed(b"gu"),
            [Intent::PickupItem, Intent::ToggleItemUse]
        );
        assert_eq!(SkillSlot::new(0), None);
        assert_eq!(SkillSlot::new(11), None);
    }

    #[test]
    fn decoder_bounds_spam_linearly_and_recovers_for_later_input() {
        let mut decoder = InputDecoder::default();
        let spam = vec![b's'; MAX_INPUT_BYTES_PER_FEED * 8];
        let intents = decoder.feed(&spam);
        assert_eq!(intents.len(), MAX_INTENTS_PER_FEED);
        assert!(intents.iter().all(|intent| *intent == Intent::Wait));
        assert_eq!(decoder.feed(b"Q"), [Intent::Quit]);

        let unknown = vec![b'h'; MAX_INPUT_BYTES_PER_FEED * 8];
        assert!(decoder.feed(&unknown).is_empty());
        assert!(decoder.feed(b"\x1b[").is_empty());
        assert_eq!(decoder.feed(b"A"), [Intent::Move(Direction::North)]);
    }

    #[test]
    fn exploration_and_victory_profiles_are_snapshot_tested() {
        let ascii = DisplayProfile {
            ascii: true,
            no_color: true,
        };
        insta::assert_snapshot!(
            "exploration_active_unicode",
            rendered_game(DisplayProfile::default(), RunStatus::Active)
        );
        insta::assert_snapshot!(
            "exploration_active_ascii",
            rendered_game(ascii, RunStatus::Active)
        );
        insta::assert_snapshot!(
            "exploration_victory_unicode",
            rendered_game(DisplayProfile::default(), RunStatus::Victory)
        );
        insta::assert_snapshot!(
            "exploration_victory_ascii",
            rendered_game(ascii, RunStatus::Victory)
        );
        insta::assert_snapshot!(
            "combat_death_unicode",
            rendered_game(DisplayProfile::default(), RunStatus::Death)
        );
        insta::assert_snapshot!("combat_death_ascii", rendered_game(ascii, RunStatus::Death));
        insta::assert_snapshot!(
            "enemy_showcase_unicode",
            rendered_enemy_showcase(DisplayProfile::default())
        );
        insta::assert_snapshot!("enemy_showcase_ascii", rendered_enemy_showcase(ascii));
        insta::assert_snapshot!(
            "exploration_help_unicode",
            rendered_game_at(80, 24, DisplayProfile::default(), RunStatus::Active, true)
        );
        insta::assert_snapshot!(
            "exploration_help_ascii",
            rendered_game_at(80, 24, ascii, RunStatus::Active, true)
        );
        insta::assert_snapshot!(
            "exploration_small_unicode",
            rendered_game_at(60, 20, DisplayProfile::default(), RunStatus::Active, false)
        );
        insta::assert_snapshot!(
            "exploration_small_ascii",
            rendered_game_at(60, 20, ascii, RunStatus::Active, false)
        );
    }

    #[test]
    fn portrait_layout_moves_status_below_map_but_landscape_keeps_side_panel() {
        let portrait =
            game_terminal_at(40, 50, DisplayProfile::default(), RunStatus::Active, false);
        let landscape =
            game_terminal_at(80, 24, DisplayProfile::default(), RunStatus::Active, false);

        let (_, portrait_status_y) = text_position(&portrait, 40, 50, "Status");
        let (_, portrait_map_y) = text_position(&portrait, 40, 50, "Ossuary");
        let (landscape_status_x, landscape_status_y) = text_position(&landscape, 80, 24, "Status");
        let (landscape_map_x, landscape_map_y) = text_position(&landscape, 80, 24, "Ossuary");

        assert!(
            portrait_status_y > portrait_map_y,
            "portrait status must follow the map"
        );
        assert_eq!(
            landscape_status_y, landscape_map_y,
            "landscape keeps panels side by side"
        );
        assert!(
            landscape_status_x > landscape_map_x,
            "landscape status remains on the right"
        );
        assert!(terminal_text(&portrait, 40, 50).contains("GRAVE KNIGHT"));
    }

    #[test]
    fn ascii_profile_never_renders_non_ascii_symbols() {
        let profile = DisplayProfile {
            ascii: true,
            no_color: true,
        };
        for output in [
            rendered_game(profile, RunStatus::Active),
            rendered_game(profile, RunStatus::Victory),
            rendered_game(profile, RunStatus::Death),
            rendered_enemy_showcase(profile),
            rendered_game_at(80, 24, profile, RunStatus::Active, true),
            rendered_game_at(60, 20, profile, RunStatus::Active, false),
        ] {
            assert!(output.is_ascii(), "strict ASCII output contained Unicode");
        }
    }

    #[test]
    fn semantic_palette_colors_representative_game_elements() {
        let profile = DisplayProfile::default();
        assert_eq!(
            semantic_style(profile, SemanticTone::Player).fg,
            Some(Color::White)
        );
        assert_eq!(
            semantic_style(profile, SemanticTone::Door).fg,
            Some(Color::Yellow)
        );
        assert_eq!(
            semantic_style(profile, SemanticTone::Exit).fg,
            Some(Color::LightYellow)
        );
        assert_eq!(
            semantic_style(profile, SemanticTone::Ghoul).fg,
            Some(Color::Green)
        );
        assert_eq!(
            semantic_style(profile, SemanticTone::Cultist).fg,
            Some(Color::Magenta)
        );
        assert_eq!(
            semantic_style(profile, SemanticTone::Lunge).fg,
            Some(Color::LightRed)
        );
        assert_eq!(
            semantic_style(profile, SemanticTone::Hex).fg,
            Some(Color::LightMagenta)
        );

        let terminal = game_terminal_at(80, 24, profile, RunStatus::Active, false);
        let player = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .find(|cell| cell.symbol() == "@")
            .expect("the rendered game contains the player");
        assert_eq!(player.fg, Color::White);
        assert!(player.modifier.contains(Modifier::BOLD));
        assert_rendered_text_color(&terminal, "HP ██████████ 32/32", Color::Green);
        assert_rendered_text_color(&terminal, "1 Cleave M3 Ready", Color::Yellow);
        assert_rendered_text_color(&terminal, "You descend into the ossuary.", Color::Gray);

        let help = game_terminal_at(80, 24, profile, RunStatus::Active, true);
        assert_rendered_text_color(&help, "Reach >.", Color::LightCyan);
        let victory = game_terminal_at(80, 24, profile, RunStatus::Victory, false);
        assert_rendered_text_color(&victory, "YOU ESCAPED THE OSSUARY", Color::LightYellow);
        let death = game_terminal_at(80, 24, profile, RunStatus::Death, false);
        assert_rendered_text_color(&death, "YOU DIED", Color::LightRed);

        let showcase = enemy_showcase_terminal(profile);
        for (symbol, expected) in [
            ("s", Color::White),
            ("G", Color::LightRed),
            ("C", Color::LightMagenta),
            ("█", Color::DarkGray),
        ] {
            assert!(
                showcase
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .any(|cell| cell.symbol() == symbol && cell.fg == expected),
                "the showcase contains {symbol} with {expected:?}"
            );
        }

        let no_color = DisplayProfile {
            no_color: true,
            ..profile
        };
        assert_eq!(
            rendered_game(profile, RunStatus::Active),
            rendered_game(no_color, RunStatus::Active),
            "color must not change rendered glyphs or labels"
        );
        assert_eq!(
            rendered_enemy_showcase(profile),
            rendered_enemy_showcase(no_color),
            "color must not change enemy or telegraph glyphs"
        );
    }

    #[test]
    fn no_color_profile_resets_all_foreground_and_background_colors() {
        let profile = DisplayProfile {
            ascii: true,
            no_color: true,
        };
        for terminal in [
            game_terminal_at(80, 24, profile, RunStatus::Active, false),
            game_terminal_at(80, 24, profile, RunStatus::Active, true),
            enemy_showcase_terminal(profile),
            game_terminal_at(60, 20, profile, RunStatus::Active, false),
            game_terminal_at(80, 24, profile, RunStatus::Victory, false),
            game_terminal_at(80, 24, profile, RunStatus::Death, false),
        ] {
            assert!(
                terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .all(|cell| { cell.fg == Color::Reset && cell.bg == Color::Reset })
            );
        }
    }

    #[test]
    fn unseen_features_are_hidden_and_explored_terrain_uses_memory_glyphs() {
        let mut game = ExplorationGame::new(Some(RunSeed(9))).unwrap();
        let exit = game.map.exit;
        game.visible.remove(&exit);
        game.explored.remove(&exit);
        game.spawn_ground_item(ItemId::HEALTH_POTION, exit);
        assert_eq!(
            exploration_glyph(&game, exit, DisplayProfile::default()),
            ' '
        );
        game.explored.insert(exit);
        assert_eq!(
            exploration_glyph(&game, exit, DisplayProfile::default()),
            ','
        );
        game.visible.insert(exit);
        assert_eq!(
            exploration_glyph(&game, exit, DisplayProfile::default()),
            '¡'
        );
    }

    #[test]
    fn partial_terminal_setup_restores_every_possible_acquisition_once() {
        use std::{cell::Cell, rc::Rc};

        #[derive(Default)]
        struct InjectedFailureWriter {
            bytes: Vec<u8>,
            fail_flush: Option<usize>,
            flushes: usize,
            fail_write_after: Option<usize>,
        }

        impl Write for InjectedFailureWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if let Some(limit) = self.fail_write_after {
                    if self.bytes.len() >= limit {
                        self.fail_write_after = None;
                        return Err(io::Error::other("injected write failure"));
                    }
                    let accepted = bytes.len().min(limit - self.bytes.len());
                    self.bytes.extend_from_slice(&bytes[..accepted]);
                    return Ok(accepted);
                }
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                self.flushes += 1;
                if self.fail_flush == Some(self.flushes) {
                    Err(io::Error::other("injected flush failure"))
                } else {
                    Ok(())
                }
            }
        }

        let mut raw_failure = LocalTerminalGuard::default();
        let mut raw_output = InjectedFailureWriter::default();
        assert!(
            raw_failure
                .setup_with(&mut raw_output, || Err(io::Error::other("raw mode")))
                .is_err()
        );
        assert_eq!(raw_failure, LocalTerminalGuard::default());
        raw_failure.restore_with(&mut raw_output, || panic!("raw mode was never acquired"));
        assert!(raw_output.bytes.is_empty());

        for (failing_flush, expected_cleanup) in [
            (1, b"\x1b[?1049l".as_slice()),
            (2, b"\x1b[?25h\x1b[?1049l".as_slice()),
        ] {
            let disabled = Rc::new(Cell::new(0));
            let disabled_for_restore = Rc::clone(&disabled);
            let mut output = InjectedFailureWriter {
                fail_flush: Some(failing_flush),
                ..InjectedFailureWriter::default()
            };
            let mut guard = LocalTerminalGuard::default();
            assert!(guard.setup_with(&mut output, || Ok(())).is_err());
            let setup_len = output.bytes.len();

            guard.restore_with(&mut output, move || {
                disabled_for_restore.set(disabled_for_restore.get() + 1);
                Ok(())
            });

            assert_eq!(disabled.get(), 1);
            assert_eq!(&output.bytes[setup_len..], expected_cleanup);
            assert_eq!(guard, LocalTerminalGuard::default());

            guard.restore_with(&mut output, || panic!("restoration must be idempotent"));
            assert_eq!(disabled.get(), 1);
        }

        let enter_sequence_len = b"\x1b[?1049h".len();
        for (fail_after, expected_cleanup) in [
            (3, b"\x1b[?1049l".as_slice()),
            (enter_sequence_len + 3, b"\x1b[?25h\x1b[?1049l".as_slice()),
        ] {
            let disabled = Rc::new(Cell::new(0));
            let disabled_for_restore = Rc::clone(&disabled);
            let mut output = InjectedFailureWriter {
                fail_write_after: Some(fail_after),
                ..InjectedFailureWriter::default()
            };
            let mut guard = LocalTerminalGuard::default();
            assert!(guard.setup_with(&mut output, || Ok(())).is_err());
            let setup_len = output.bytes.len();

            guard.restore_with(&mut output, move || {
                disabled_for_restore.set(disabled_for_restore.get() + 1);
                Ok(())
            });

            assert_eq!(disabled.get(), 1);
            assert_eq!(&output.bytes[setup_len..], expected_cleanup);
            assert_eq!(guard, LocalTerminalGuard::default());
            guard.restore_with(&mut output, || panic!("restoration must be idempotent"));
            assert_eq!(disabled.get(), 1);
        }
    }

    #[test]
    fn dimensions_are_capped() {
        assert_eq!(
            capped_area(u32::MAX, u32::MAX),
            Rect::new(0, 0, MAX_WIDTH, MAX_HEIGHT)
        );
        assert_eq!(capped_area(0, 0), Rect::new(0, 0, 1, 1));
    }

    #[test]
    fn gameplay_intents_are_ignored_below_minimum_size() {
        let small = Size::new(60, 20);
        assert!(intent_allowed_at_size(Intent::Wait, small));
        assert!(intent_allowed_at_size(Intent::Quit, small));
    }

    #[test]
    fn skill_slots_dispatch_cleave_and_empty_slots_are_free() {
        let mut empty_slots = ExplorationGame::new(Some(RunSeed(33))).unwrap();
        empty_slots.start();
        let unchanged = empty_slots.clone();
        for number in 3..=10 {
            assert!(
                apply_game_intent(
                    &mut empty_slots,
                    Intent::UseSkill(SkillSlot::new(number).unwrap()),
                )
                .unwrap()
            );
        }
        assert_eq!(empty_slots, unchanged);

        let mut cleave = ExplorationGame::new(Some(RunSeed(44))).unwrap();
        cleave.start();
        let adjacent = [
            Direction::North,
            Direction::East,
            Direction::South,
            Direction::West,
        ]
        .into_iter()
        .map(|direction| {
            let (dx, dy) = direction.delta();
            cleave.player.position.offset(dx, dy)
        })
        .find(|position| cleave.map.tile(*position).is_some_and(Tile::is_walkable))
        .expect("the generated start has an adjacent floor");
        cleave.hostiles.clear();
        cleave.spawn_hostile(ActorKind::Skeleton, adjacent, 20, 1, 1);

        assert!(apply_game_intent(&mut cleave, Intent::UseSkill(SkillSlot::CLEAVE)).unwrap());
        assert_eq!(cleave.turn, 1);
        assert_eq!(
            cleave.abilities[0].as_ref().unwrap().cooldown_remaining,
            crate::game::CLEAVE_COOLDOWN_TURNS
        );
        assert!(
            cleave
                .events
                .iter()
                .any(|event| event == "You unleash Cleave.")
        );
        let on_cooldown = cleave.clone();
        assert!(apply_game_intent(&mut cleave, Intent::UseSkill(SkillSlot::CLEAVE)).unwrap());
        assert_eq!(cleave, on_cooldown);
    }

    #[test]
    fn shared_game_intents_keep_sessions_independent() {
        let mut waiting = ExplorationGame::new(Some(RunSeed(11))).unwrap();
        let mut helping = ExplorationGame::new(Some(RunSeed(22))).unwrap();
        waiting.start();
        helping.start();

        assert!(apply_game_intent(&mut waiting, Intent::Wait).unwrap());
        assert!(apply_game_intent(&mut helping, Intent::ToggleHelp).unwrap());

        assert_eq!(waiting.turn, 1);
        assert!(!waiting.help);
        assert_eq!(helping.turn, 0);
        assert!(helping.help);
        assert_ne!(waiting.seed(), helping.seed());
    }
    #[test]
    fn resource_bars_clamp_and_support_ascii_and_unicode() {
        let unicode =
            resource_bar_line("HP", 5, 10, DisplayProfile::default(), SemanticTone::Health);
        assert_eq!(
            unicode
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "HP █████░░░░░ 5/10"
        );
        let ascii = resource_bar_line(
            "MP",
            -3,
            0,
            DisplayProfile {
                ascii: true,
                no_color: true,
            },
            SemanticTone::Candidate,
        );
        assert_eq!(
            ascii
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "MP [----------] 0/0"
        );
        let over = resource_bar_line(
            "HP",
            99,
            10,
            DisplayProfile {
                ascii: true,
                no_color: true,
            },
            SemanticTone::Health,
        );
        assert_eq!(
            over.iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "HP [##########] 10/10"
        );
    }
}
