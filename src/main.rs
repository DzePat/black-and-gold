#[macro_use]
extern crate lazy_static;

use std::f32::consts::PI;

use macroquad::audio::{
    load_sound, play_sound, play_sound_once, set_sound_volume, stop_sound, PlaySoundParams, Sound,
};

use macroquad::experimental::animation::{AnimatedSprite, Animation};
use macroquad::experimental::collections::storage;
use macroquad::experimental::coroutines::start_coroutine;
use macroquad::prelude::*;
use macroquad::rand::ChooseRandom;
use macroquad::ui::{hash, root_ui, Skin};
use macroquad_particles::{self as particles, AtlasConfig, Emitter, EmitterConfig};

use log::Log;
use log::{Level, LevelFilter, Metadata, Record};

#[cfg(target_os = "ios")]
use oslog::OsLogger;

struct SimpleLogger;

impl Log for SimpleLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            {
                println!("{} - {}", record.level(), record.args());
            }
        }
    }

    fn flush(&self) {}
}

const GAME_TITLE: &str = "¡AFUERA!";
const MOVEMENT_SPEED: f32 = 400.0;
const STARFIELD_SPEED: f32 = 0.01;
const BALL_RADIUS: f32 = 16.0;
const MAX_BULLETS_PER_SECOND: f64 = 4.0;

const FRAGMENT_SHADER: &str = include_str!("starfield-shader.glsl");

const VERTEX_SHADER: &str = "#version 100
attribute vec3 position;
// attribute vec2 texcoord;
// attribute vec4 color0;
varying float iTime;

uniform mat4 Model;
uniform mat4 Projection;
uniform vec4 _Time;

void main() {
    gl_Position = Projection * Model * vec4(position, 1);
    iTime = _Time.x;
}
";

fn save_high_score(score: u32) {
    let storage = &mut quad_storage::STORAGE.lock().unwrap();
    storage.set("highscore", &score.to_string());
}

fn load_high_score() -> u32 {
    let storage = &mut quad_storage::STORAGE.lock().unwrap();
    storage
        .get("highscore")
        .unwrap_or("0".to_string())
        .parse::<u32>()
        .unwrap()
}

lazy_static! {
    static ref ENEMY_COLORS: Vec<Color> = vec![
        BEIGE, BLUE, BROWN, DARKBLUE, DARKBROWN, DARKGRAY, DARKGREEN, DARKPURPLE, GRAY, GREEN,
        LIME, MAGENTA, MAROON, ORANGE, PINK, PURPLE, RED, SKYBLUE, VIOLET, YELLOW,
    ];
}

struct Shape {
    size: f32,
    speed: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Color,
    collided: bool,
}

impl Shape {
    fn collides_with_circle(&self, circle: &Shape) -> bool {
        let half = self.size / 2.0;
        let dx = (self.x - circle.x).abs().max(half) - half;
        let dy = (self.y - circle.y).abs().max(half) - half;
        dx * dx + dy * dy <= circle.size * circle.size / 4.0
    }
    fn collides_with(&self, other: &Self) -> bool {
        self.rect().overlaps(&other.rect())
    }

    fn rect(&self) -> Rect {
        Rect {
            x: self.x - self.size / 2.0,
            y: self.y - self.size / 2.0,
            w: self.w,
            h: self.h,
        }
    }
}

struct Enemy {
    id: usize,
    shape: Shape,
    bullet_count: usize,
}
struct EnemyBullet {
    enemy_id: usize,
    shape: Shape,
}

enum GameState {
    MainMenu,
    Playing,
    Paused,
    GameOver,
}

fn oscillating_alpha(base_color: Color, cycles_per_second: f32) -> Color {
    let alpha = 0.5 * (1.0 + f32::sin(cycles_per_second * get_time() as f32 * PI / 2.0));
    Color::new(base_color.r, base_color.g, base_color.b, alpha)
}

fn draw_game_title() {
    let text = GAME_TITLE;
    let font_size = 144;
    let text_dimensions = measure_text(text, None, font_size, 1.0);
    draw_text(
        text,
        screen_width() / 2.0 - text_dimensions.width / 2.0,
        screen_height() / 4.0,
        font_size as f32,
        GOLD,
    );
}

fn particle_explosion() -> particles::EmitterConfig {
    particles::EmitterConfig {
        local_coords: false,
        one_shot: true,
        emitting: true,
        lifetime: 0.6,
        lifetime_randomness: 0.3,
        explosiveness: 0.65,
        initial_direction_spread: 2.0 * std::f32::consts::PI,
        initial_velocity: 400.0,
        initial_velocity_randomness: 0.8,
        size: 12.0,
        size_randomness: 0.3,
        atlas: Some(AtlasConfig::new(5, 1, 0..)),
        // colors_curve: ColorCurve {
        //     start: RED,
        //     mid: ORANGE,
        //     end: RED,
        // },
        ..Default::default()
    }
}

struct Resources {
    ship_texture: Texture2D,
    bullet_texture: Texture2D,
    explosion_texture: Texture2D,
    enemy_small_texture: Texture2D,
    theme_music: Sound,
    sound_explosion: Sound,
    sound_laser: Sound,
    ui_skin: Skin,
}

impl Resources {
    async fn new() -> Result<Resources, macroquad::Error> {
        let ship_texture: Texture2D = load_texture("ship.png").await?;
        ship_texture.set_filter(FilterMode::Nearest);
        let bullet_texture: Texture2D = load_texture("laser-bolts.png").await?;
        bullet_texture.set_filter(FilterMode::Nearest);
        let explosion_texture: Texture2D = load_texture("explosion.png").await?;
        explosion_texture.set_filter(FilterMode::Nearest);
        let enemy_small_texture: Texture2D = load_texture("enemy-small.png").await?;
        enemy_small_texture.set_filter(FilterMode::Nearest);

        let theme_music = load_sound("8bit-spaceshooter.ogg").await?;
        let sound_explosion = load_sound("explosion.wav").await?;
        let sound_laser = load_sound("laser.wav").await?;

        let window_background = load_image("window_background.png").await?;
        let button_background = load_image("button_background.png").await?;
        let button_clicked_background = load_image("button_clicked_background.png").await?;
        let font = load_file("atari_games.ttf").await?;

        let window_style = root_ui()
            .style_builder()
            .background(window_background)
            .background_margin(RectOffset::new(32.0, 76.0, 44.0, 20.0))
            .margin(RectOffset::new(0.0, -40.0, 0.0, 0.0))
            .build();
        let button_style = root_ui()
            .style_builder()
            .background(button_background)
            .background_clicked(button_clicked_background)
            .background_margin(RectOffset::new(16.0, 16.0, 16.0, 16.0))
            .margin(RectOffset::new(16.0, 0.0, -8.0, -8.0))
            .font(&font)?
            .text_color(WHITE)
            .font_size(64)
            .build();
        let label_style = root_ui()
            .style_builder()
            .font(&font)?
            .text_color(WHITE)
            .font_size(28)
            .build();
        let ui_skin = Skin {
            window_style,
            button_style,
            label_style,
            ..root_ui().default_skin()
        };

        Ok(Resources {
            ship_texture,
            bullet_texture,
            explosion_texture,
            enemy_small_texture,
            theme_music,
            sound_explosion,
            sound_laser,
            ui_skin,
        })
    }

    pub async fn load() -> Result<(), macroquad::Error> {
        let resources_loading = start_coroutine(async move {
            let resources = Resources::new().await.unwrap();
            storage::store(resources);
        });

        while !resources_loading.is_done() {
            clear_background(BLACK);
            let text = format!(
                "Loading resources {}",
                ".".repeat(((get_time() * 2.) as usize) % 4)
            );
            draw_text(
                &text,
                screen_width() / 2. - 160.,
                screen_height() / 2.,
                40.,
                WHITE,
            );
            next_frame().await;
        }

        Ok(())
    }
}

fn draw_game_objects(
    enemies: &[Enemy],
    bullets: &[Shape],
    enemy_bullets: &[EnemyBullet],
    circle: &Shape,
    explosions: &mut [(Emitter, Vec2)],
    bullet_sprite: &AnimatedSprite,
    enemy_bullet_sprite: &AnimatedSprite,
    ship_sprite: &AnimatedSprite,
    enemy_small_sprite: &AnimatedSprite,
    resources: &Resources,
) {
    let enemy_frame: animation::AnimationFrame = enemy_small_sprite.frame();
    for enemy in enemies {
        draw_texture_ex(
            &resources.enemy_small_texture,
            enemy.shape.x - enemy.shape.size / 2.0,
            enemy.shape.y - enemy.shape.size / 2.0,
            WHITE, // square.color,
            DrawTextureParams {
                dest_size: Some(vec2(enemy.shape.size, enemy.shape.size)),
                source: Some(enemy_frame.source_rect),
                ..Default::default()
            },
        );
    }

    let bullet_frame = enemy_bullet_sprite.frame();
    for bullet in enemy_bullets {
        draw_texture_ex(
            &resources.bullet_texture,
            bullet.shape.x - bullet.shape.size / 2.0,
            bullet.shape.y - bullet.shape.size / 2.0,
            bullet.shape.color,
            DrawTextureParams {
                dest_size: Some(vec2(bullet.shape.size, bullet.shape.size)),
                source: Some(bullet_frame.source_rect),
                rotation: PI,
                ..Default::default()
            },
        );
    }

    let bullet_frame = bullet_sprite.frame();
    for bullet in bullets {
        draw_texture_ex(
            &resources.bullet_texture,
            bullet.x - bullet.size / 2.0,
            bullet.y - bullet.size / 2.0,
            bullet.color,
            DrawTextureParams {
                dest_size: Some(vec2(bullet.size, bullet.size)),
                source: Some(bullet_frame.source_rect),
                ..Default::default()
            },
        );
    }

    let ship_frame = ship_sprite.frame();
    draw_texture_ex(
        &resources.ship_texture,
        circle.x - ship_frame.dest_size.x,
        circle.y - ship_frame.dest_size.y,
        WHITE,
        DrawTextureParams {
            dest_size: Some(ship_frame.dest_size * 2.0),
            source: Some(ship_frame.source_rect),
            ..Default::default()
        },
    );

    for (explosion, coords) in explosions.iter_mut() {
        explosion.draw(*coords);
    }
}

fn draw_score(score: u32, high_score: u32, high_score_beaten: bool) {
    draw_text(format!("Score: {}", score).as_str(), 10.0, 35.0, 25.0, GOLD);
    let high_score_text = format!("High score: {}", high_score);
    let high_score_beaten_text = if high_score_beaten {
        "New high score!"
    } else {
        ""
    };

    let text_dimensions = measure_text(high_score_text.as_str(), None, 25, 1.0);
    draw_text(
        high_score_text.as_str(),
        screen_width() - text_dimensions.width - 10.0,
        35.0,
        25.0,
        GOLD,
    );

    if high_score_beaten {
        let text_dimensions = measure_text(high_score_beaten_text, None, 25, 1.0);
        draw_text(
            high_score_beaten_text,
            screen_width() - text_dimensions.width - 10.0,
            35.0 + text_dimensions.height + text_dimensions.offset_y,
            25.0,
            oscillating_alpha(GOLD, 3.0),
        );
    }
}

#[macroquad::main("¡Viva la libertad, CARAJO!")]
async fn main() -> Result<(), macroquad::Error> {
    rand::srand(miniquad::date::now() as u64);

    #[cfg(not(target_os = "ios"))]
    {
        log::set_boxed_logger(Box::new(SimpleLogger))
            .map(|()| log::set_max_level(LevelFilter::Info))
            .expect("Failed to set logger");
    }

    #[cfg(target_os = "ios")]
    {
        OsLogger::new("com.mittspel")
            .level_filter(LevelFilter::Debug)
            .init()
            .unwrap();
    }

    log::info!("BOOM!");

    let base_width = 750.0;
    let base_enemies = 30;

    let mut score: u32 = 0;
    let mut high_score: u32 = load_high_score();
    let mut high_score_beaten = false;

    let mut last_bullet_time = get_time();
    let mut enemies = vec![];
    let mut next_enemy_id = 0;
    let mut bullets: Vec<Shape> = vec![];
    let mut enemy_bullets: Vec<EnemyBullet> = vec![];

    let mut direction_modifier: f32 = 0.0;
    let render_target = render_target(320, 150);
    render_target.texture.set_filter(FilterMode::Nearest);
    let material = load_material(
        ShaderSource::Glsl {
            vertex: VERTEX_SHADER,
            fragment: FRAGMENT_SHADER,
        },
        MaterialParams {
            uniforms: vec![
                ("iResolution".to_owned(), UniformType::Float2),
                ("direction_modifier".to_owned(), UniformType::Float1),
            ],
            ..Default::default()
        },
    )?;

    let mut explosions: Vec<(Emitter, Vec2)> = vec![];

    let mut game_state = GameState::MainMenu;

    set_pc_assets_folder("assets");
    Resources::load().await?;
    let resources = storage::get::<Resources>();

    play_sound(
        &resources.theme_music,
        PlaySoundParams {
            looped: true,
            volume: 0.1,
        },
    );

    let mut ship_sprite = AnimatedSprite::new(
        16,
        24,
        &[
            Animation {
                name: "idle".to_string(),
                row: 0,
                frames: 2,
                fps: 12,
            },
            Animation {
                name: "left1".to_string(),
                row: 1,
                frames: 2,
                fps: 12,
            },
            Animation {
                name: "left2".to_string(),
                row: 2,
                frames: 2,
                fps: 12,
            },
            Animation {
                name: "right1".to_string(),
                row: 3,
                frames: 2,
                fps: 12,
            },
            Animation {
                name: "right2".to_string(),
                row: 4,
                frames: 2,
                fps: 12,
            },
        ],
        true,
    );

    let mut left_direction_time = get_time();
    let mut right_direction_time = get_time();

    let circle_size = BALL_RADIUS * 2.0;
    let ship_sprite_w = ship_sprite.frame().source_rect.w;
    let ship_sprite_h = ship_sprite.frame().source_rect.h;
    let mut circle = Shape {
        size: circle_size,
        speed: MOVEMENT_SPEED,
        x: screen_width() / 2.0,
        y: screen_height() / 2.0,
        w: ship_sprite_w * circle_size / ship_sprite_w,
        h: ship_sprite_h * circle_size / ship_sprite_h,
        color: GOLD,
        collided: false,
    };

    let mut bullet_sprite = AnimatedSprite::new(
        16,
        16,
        &[Animation {
            name: "bolt".to_string(),
            row: 1,
            frames: 2,
            fps: 12,
        }],
        true,
    );
    bullet_sprite.set_animation(0);

    let mut enemy_bullet_sprite = AnimatedSprite::new(
        16,
        16,
        &[Animation {
            name: "bolt".to_string(),
            row: 1,
            frames: 2,
            fps: 12,
        }],
        true,
    );
    enemy_bullet_sprite.set_animation(0);

    let mut enemy_small_sprite = AnimatedSprite::new(
        17,
        16,
        &[Animation {
            name: "enemy_small".to_string(),
            row: 0,
            frames: 2,
            fps: 12,
        }],
        true,
    );

    root_ui().push_skin(&resources.ui_skin);
    let window_size = vec2(370.0, 320.0);

    let mut has_valid_mouse_position = false;

    loop {
        clear_background(BLACK);

        let screen_width = screen_width();
        let screen_height = screen_height();
        let scale_x = screen_width / base_width;
        let scale = scale_x;

        let max_enemies = (base_enemies as f32 * scale).floor() as usize;

        material.set_uniform("iResolution", (screen_width, screen_height));
        material.set_uniform("direction_modifier", direction_modifier);

        gl_use_material(&material);
        draw_texture_ex(
            &render_target.texture,
            0.,
            0.,
            WHITE,
            DrawTextureParams {
                dest_size: Some(vec2(screen_width, screen_height)),
                ..Default::default()
            },
        );
        gl_use_default_material();

        let mut exit_game = false;

        match game_state {
            GameState::MainMenu => {
                set_sound_volume(&resources.theme_music, 0.2);
                score = 0;
                high_score_beaten = false;
                root_ui().window(
                    hash!(),
                    vec2(
                        screen_width / 2.0 - window_size.x / 2.0,
                        screen_height / 2.0 - window_size.y / 2.0,
                    ),
                    window_size,
                    |ui| {
                        ui.label(vec2(90.0, -34.0), "Main menu");
                        if ui.button(vec2(66.0, 25.0), "Play") {
                            enemies.clear();
                            bullets.clear();
                            enemy_bullets.clear();
                            explosions.clear();
                            circle.x = screen_width / 2.0;
                            circle.y = screen_height - circle.size;
                            game_state = GameState::Playing;
                            has_valid_mouse_position = false;
                        }
                        if ui.button(vec2(66.0, 125.0), "Exit") {
                            exit_game = true;
                        }
                    },
                );
                draw_game_title();
                draw_score(score, high_score, high_score_beaten);
            }
            GameState::Playing => {
                set_sound_volume(&resources.theme_music, 1.0);
                if is_key_pressed(KeyCode::Escape) {
                    game_state = GameState::Paused;
                }
                let delta_time = get_frame_time();
                let my_movement_speed = delta_time * MOVEMENT_SPEED;
                let star_movement_speed = delta_time * STARFIELD_SPEED;

                #[cfg(target_os = "ios")]
                if is_mouse_button_pressed(MouseButton::Left) {
                    has_valid_mouse_position = true;
                }
                #[cfg(target_os = "ios")]
                let (mouse_x, mouse_y) = if has_valid_mouse_position {
                    mouse_position()
                } else {
                    (circle.x, circle.y)
                };

                #[cfg(target_os = "ios")]
                let dir_x = mouse_x - circle.x;
                #[cfg(target_os = "ios")]
                let dir_y = mouse_y - circle.y;
                #[cfg(not(target_os = "ios"))]
                let dir_x: f32 = if is_key_down(KeyCode::Left) {
                    -MOVEMENT_SPEED
                } else if is_key_down(KeyCode::Right) {
                    MOVEMENT_SPEED
                } else {
                    0.0
                };
                #[cfg(not(target_os = "ios"))]
                let dir_y: f32 = if is_key_down(KeyCode::Up) {
                    -MOVEMENT_SPEED
                } else if is_key_down(KeyCode::Down) {
                    MOVEMENT_SPEED
                } else {
                    0.0
                };

                ship_sprite.set_animation(0);
                if is_key_pressed(KeyCode::Left) {
                    left_direction_time = get_time();
                }
                if dir_x < 0.0 {
                    circle.x -= my_movement_speed.min(dir_x.abs());
                    direction_modifier -= star_movement_speed;
                    ship_sprite.set_animation(if get_time() < left_direction_time + 0.5 {
                        1
                    } else {
                        2
                    });
                }
                if is_key_pressed(KeyCode::Right) {
                    right_direction_time = get_time();
                }
                if dir_x > 0.0 {
                    circle.x += my_movement_speed.min(dir_x);
                    direction_modifier += star_movement_speed;
                    ship_sprite.set_animation(if get_time() < right_direction_time + 0.5 {
                        3
                    } else {
                        4
                    });
                }
                if dir_y > 0.0 {
                    circle.y += my_movement_speed.min(dir_y);
                }
                if dir_y < 0.0 {
                    circle.y -= my_movement_speed.min(dir_y.abs());
                }

                circle.x = circle
                    .x
                    .min(screen_width - BALL_RADIUS)
                    .max(0.0 + BALL_RADIUS);
                circle.y = circle
                    .y
                    .min(screen_height - BALL_RADIUS)
                    .max(0.0 + BALL_RADIUS);

                if get_time() - last_bullet_time > 1.0 / MAX_BULLETS_PER_SECOND {
                    last_bullet_time = get_time();
                    let size = 32.0;
                    let bullet_sprite_w = bullet_sprite.frame().source_rect.w;
                    let bullet_sprite_h = bullet_sprite.frame().source_rect.h;
                    let w = bullet_sprite_w * size / bullet_sprite_w;
                    let h = bullet_sprite_h * size / bullet_sprite_h;
                    bullets.push(Shape {
                        x: circle.x,
                        y: circle.y - 24.0,
                        w,
                        h,
                        speed: circle.speed * 2.0,
                        color: GOLD,
                        size,
                        collided: false,
                    });
                    play_sound_once(&resources.sound_laser);
                }

                if enemies.len() < max_enemies && rand::gen_range(0, 99) >= 95 {
                    let size = rand::gen_range(16.0, 64.0) * scale;
                    let ship_sprite_w = enemy_small_sprite.frame().source_rect.w;
                    let ship_sprite_h = enemy_small_sprite.frame().source_rect.h;
                    let w = ship_sprite_w * size / ship_sprite_w;
                    let h = ship_sprite_h * size / ship_sprite_h;
                    enemies.push(Enemy {
                        id: next_enemy_id,
                        bullet_count: 0,
                        shape: Shape {
                            size,
                            speed: rand::gen_range(50.0, 150.0),
                            x: rand::gen_range(size / 2.0, screen_width - size / 2.0),
                            y: -size,
                            w,
                            h,
                            color: *ENEMY_COLORS.choose().unwrap(),
                            collided: false,
                        },
                    });
                    next_enemy_id += 1;
                }

                for enemy in &mut enemies {
                    enemy.shape.y += enemy.shape.speed * delta_time;
                }
                for bullet in &mut bullets {
                    bullet.y -= bullet.speed * delta_time;
                }
                for bullet in &mut enemy_bullets {
                    bullet.shape.y += bullet.shape.speed * delta_time;
                }

                ship_sprite.update();
                bullet_sprite.update();
                enemy_small_sprite.update();

                if enemies
                    .iter()
                    .any(|enemy| enemy.shape.collides_with_circle(&circle))
                {
                    if score == high_score {
                        save_high_score(score);
                    }
                    game_state = GameState::GameOver;
                }

                for enemy in enemies.iter_mut() {
                    for bullet in bullets.iter_mut() {
                        if bullet.collides_with(&enemy.shape) {
                            bullet.collided = true;
                            enemy.shape.collided = true;
                            score += enemy.shape.size.round() as u32;
                            if score > high_score {
                                high_score_beaten = true;
                                high_score = score;
                            }
                            explosions.push((
                                Emitter::new(EmitterConfig {
                                    amount: enemy.shape.size.round() as u32 * 2,
                                    texture: Some(resources.explosion_texture.clone()),
                                    ..particle_explosion()
                                }),
                                vec2(bullet.x, bullet.y),
                            ));
                            play_sound_once(&resources.sound_explosion);
                        }
                    }
                    if circle.x > enemy.shape.x - enemy.shape.w / 2.0
                        && circle.x < enemy.shape.x + enemy.shape.w / 2.0
                        && enemy.bullet_count < 1
                    {
                        let size = 16.0;
                        let enemy_bullet_sprite_w = enemy_bullet_sprite.frame().source_rect.w;
                        let enemy_bullet_sprite_h = enemy_bullet_sprite.frame().source_rect.h;
                        let w = enemy_bullet_sprite_w * size / enemy_bullet_sprite_w;
                        let h = enemy_bullet_sprite_h * size / enemy_bullet_sprite_h;
                        enemy_bullets.push(EnemyBullet {
                            enemy_id: enemy.id,
                            shape: Shape {
                                x: enemy.shape.x,
                                y: enemy.shape.y + enemy.shape.size / 2.0,
                                w,
                                h,
                                speed: enemy.shape.speed * 3.0,
                                color: RED,
                                size,
                                collided: false,
                            },
                        });
                        enemy.bullet_count += 1;
                    }
                }

                for bullet in enemy_bullets.iter_mut() {
                    if bullet.shape.collides_with(&circle) {
                        if score == high_score {
                            save_high_score(score);
                        }
                        game_state = GameState::GameOver;
                    }
                }

                enemy_bullets.retain(|bullet| {
                    let should_keep = bullet.shape.y < screen_height + bullet.shape.size;
                    if !should_keep {
                        if let Some(enemy) =
                            enemies.iter_mut().find(|enemy| enemy.id == bullet.enemy_id)
                        {
                            enemy.bullet_count -= 1;
                        }
                    }
                    should_keep
                });

                enemies.retain(|enemy| enemy.shape.y < screen_height + enemy.shape.size);
                bullets.retain(|bullet| bullet.y > 0.0 - bullet.size / 2.0);
                enemies.retain(|enemy| !enemy.shape.collided);
                bullets.retain(|bullet| !bullet.collided);
                explosions.retain(|(explosion, _)| explosion.config.emitting);

                draw_game_objects(
                    &enemies,
                    &bullets,
                    &enemy_bullets,
                    &circle,
                    &mut explosions,
                    &bullet_sprite,
                    &enemy_bullet_sprite,
                    &ship_sprite,
                    &enemy_small_sprite,
                    &resources,
                );
                draw_score(score, high_score, high_score_beaten);
            }
            GameState::Paused => {
                stop_sound(&resources.theme_music);
                if is_key_pressed(KeyCode::Space) {
                    play_sound(
                        &resources.theme_music,
                        PlaySoundParams {
                            looped: true,
                            volume: 1.,
                        },
                    );
                    game_state = GameState::Playing;
                }
                draw_game_objects(
                    &enemies,
                    &bullets,
                    &enemy_bullets,
                    &circle,
                    &mut explosions,
                    &bullet_sprite,
                    &enemy_bullet_sprite,
                    &ship_sprite,
                    &enemy_small_sprite,
                    &resources,
                );
                draw_score(score, high_score, high_score_beaten);
                let text = "Paused";
                let text_dimensions = measure_text(text, None, 32, 1.0);
                draw_text(
                    text,
                    screen_width / 2.0 - text_dimensions.width / 2.0,
                    screen_height / 2.0,
                    32.0,
                    GOLD,
                );
                draw_game_title();
            }
            GameState::GameOver => {
                set_sound_volume(&resources.theme_music, 0.2);
                if is_key_pressed(KeyCode::Space) || is_key_pressed(KeyCode::Escape) {
                    game_state = GameState::MainMenu;
                }
                draw_game_objects(
                    &enemies,
                    &bullets,
                    &enemy_bullets,
                    &circle,
                    &mut explosions,
                    &bullet_sprite,
                    &enemy_bullet_sprite,
                    &ship_sprite,
                    &enemy_small_sprite,
                    &resources,
                );
                draw_score(score, high_score, high_score_beaten);
                let game_over_text = "GAME OVER!";
                let text_dimensions = measure_text(game_over_text, None, 32, 1.0);

                let text_x = (screen_width - text_dimensions.width) / 2.0;
                let text_y =
                    screen_height / 2.0 - text_dimensions.offset_y + text_dimensions.height;

                draw_text(game_over_text, text_x, text_y, 32.0, GOLD);
                draw_game_title();
            }
        }
        if exit_game {
            return Ok(());
        }
        next_frame().await
    }
}
